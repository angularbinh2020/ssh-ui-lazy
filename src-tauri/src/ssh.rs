use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use russh::client::{self, AuthResult, Handle, KeyboardInteractiveAuthResponse, Msg};
use russh::keys::PublicKeyOrCertificate;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect};
use zeroize::Zeroizing;

use tauri::Emitter;

use crate::crypto;
use crate::error::{AppError, AppResult};
use crate::kubectl;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);
const TCP_AUTH_TIMEOUT: Duration = Duration::from_secs(10);
const LOG_START_DETECT: Duration = Duration::from_secs(3);
const STREAM_FLUSH: Duration = Duration::from_millis(100);
const MAX_BUF: usize = 4 * 1024 * 1024;

// ---------------------------------------------------------------- marker engine

fn contains(buf: &[u8], pat: &[u8]) -> bool {
    if pat.is_empty() || pat.len() > buf.len() {
        return false;
    }
    buf.windows(pat.len()).any(|w| w == pat)
}

fn find_sub(buf: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    if from >= buf.len() || pat.len() > buf.len() - from {
        return None;
    }
    buf[from..]
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|p| p + from)
}

/// Token phải nằm ở **đầu dòng** — chống khớp nhầm với dòng echo lệnh ta gửi
/// (`echo __LAZY_...` không bắt đầu bằng token nên không bị khớp).
pub fn has_line(buf: &[u8], token: &[u8]) -> bool {
    let mut from = 0usize;
    while let Some(pos) = find_sub(buf, from, token) {
        let at_start = pos == 0 || matches!(buf[pos - 1], b'\n' | b'\r');
        if at_start {
            return true;
        }
        from = pos + 1;
    }
    false
}

/// Phát hiện chuỗi lỗi trong output (mục 5 PLAN). Trả về lỗi đã phân loại.
pub fn classify_error(buf: &[u8]) -> Option<AppError> {
    if contains(buf, b"Sorry, try again")
        || contains(buf, b"incorrect password attempts")
        || contains(buf, b"sudo: no password was provided")
    {
        return Some(AppError::WrongPassword);
    }
    if contains(buf, b"not in the sudoers file") {
        return Some(AppError::SudoFailed {
            detail: "user không có quyền sudo (không có trong sudoers)".into(),
        });
    }
    if contains(buf, b"sudo: command not found") {
        return Some(AppError::SudoFailed { detail: "không có lệnh sudo trên máy chủ".into() });
    }
    if contains(buf, b"su: Authentication failure")
        || contains(buf, b"su: Permission denied")
        || contains(buf, b"su: User account has expired")
    {
        return Some(AppError::SuFailed { detail: "xác thực su thất bại".into() });
    }
    None
}

fn looks_like_password_prompt(buf: &[u8]) -> bool {
    contains(buf, b"[sudo] password for") || contains(buf, b"Password:")
}

fn drain_old(buf: &mut Vec<u8>) {
    if buf.len() > MAX_BUF {
        buf.drain(..buf.len() / 2);
    }
}

// ---------------------------------------------------------------- host key (TOFU)

#[derive(Debug, Clone, PartialEq)]
enum ServerKeyOutcome {
    Pending,
    Accepted(String),
    Rejected(String),
}

pub struct ClientHandler {
    expected_fp: Option<String>,
    trust_new_host: bool,
    outcome: Arc<Mutex<ServerKeyOutcome>>,
}

fn fingerprint_of(key: &PublicKeyOrCertificate) -> String {
    match key {
        PublicKeyOrCertificate::PublicKey { key, .. } => {
            format!("{}", key.fingerprint(russh::keys::HashAlg::Sha256))
        }
        PublicKeyOrCertificate::Certificate(cert) => {
            format!("{}", cert.public_key().fingerprint(russh::keys::HashAlg::Sha256))
        }
    }
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fp = fingerprint_of(server_public_key);
        let mut out = self.outcome.lock().unwrap();
        match &self.expected_fp {
            Some(expected) if *expected == fp => {
                *out = ServerKeyOutcome::Accepted(fp);
                Ok(true)
            }
            Some(expected) => {
                *out = ServerKeyOutcome::Rejected(fp);
                let _ = expected;
                Ok(false)
            }
            None => {
                if self.trust_new_host {
                    *out = ServerKeyOutcome::Accepted(fp);
                    Ok(true)
                } else {
                    *out = ServerKeyOutcome::Rejected(fp);
                    Ok(false)
                }
            }
        }
    }
}

fn map_russh_err(e: russh::Error) -> AppError {
    use russh::Error as E;
    match e {
        E::ConnectionTimeout | E::KeepaliveTimeout | E::InactivityTimeout => AppError::Timeout {
            stage: "kết nối SSH".into(),
        },
        E::UnknownKey | E::KeyChanged { .. } => AppError::FingerprintChanged {
            expected: String::new(),
            actual: String::new(),
        },
        E::HUP | E::Disconnect => AppError::ConnectionLost,
        other => AppError::Ssh { detail: other.to_string() },
    }
}

pub struct ConnectParams<'a> {
    pub host: &'a str,
    pub port: u16,
    pub username: &'a str,
    pub password: &'a str,
    pub expected_fp: Option<String>,
    pub trust_new_host: bool,
}

pub struct ConnectedSession {
    pub handle: Handle<ClientHandler>,
    pub server_fingerprint: String,
}

fn client_config() -> Arc<client::Config> {
    let mut config = client::Config::default();
    config.keepalive_interval = Some(Duration::from_secs(30)); // mục 5 PLAN
    config.keepalive_max = 3; // fail 3 lần liên tiếp → mất kết nối
    config.inactivity_timeout = None;
    Arc::new(config)
}

/// Kết nối + auth (password trước, fallback keyboard-interactive) — mục 5 PLAN.
pub async fn connect_and_auth(p: ConnectParams<'_>) -> AppResult<ConnectedSession> {
    let outcome = Arc::new(Mutex::new(ServerKeyOutcome::Pending));
    let handler = ClientHandler {
        expected_fp: p.expected_fp.clone(),
        trust_new_host: p.trust_new_host,
        outcome: outcome.clone(),
    };

    let connect_fut = client::connect(
        client_config(),
        (p.host.to_string(), p.port),
        handler,
    );
    let mut handle = match tokio::time::timeout(TCP_AUTH_TIMEOUT, connect_fut).await {
        Err(_) => {
            return Err(AppError::Timeout {
                stage: format!("kết nối TCP tới {}:{}", p.host, p.port),
            })
        }
        Ok(Err(_e)) => {
            let outcome_val = outcome.lock().unwrap().clone();
            return match outcome_val {
                ServerKeyOutcome::Rejected(fp) if p.expected_fp.is_some() => {
                    Err(AppError::FingerprintChanged {
                        expected: p.expected_fp.unwrap(),
                        actual: fp,
                    })
                }
                ServerKeyOutcome::Rejected(fp) => Err(AppError::HostUnknown { fingerprint: fp }),
                _ => Err(AppError::HostUnreachable {
                    host: p.host.into(),
                    port: p.port,
                    detail: "máy chủ từ chối hoặc không thể bắt tay SSH".into(),
                }),
            };
        }
        Ok(Ok(h)) => h,
    };

    // Auth: password trước; server chỉ hỗ trợ keyboard-interactive → trả password qua đó.
    let auth = handle
        .authenticate_password(p.username, p.password)
        .await
        .map_err(map_russh_err)?;
    match auth {
        AuthResult::Success => {}
        AuthResult::Failure { .. } => {
            let ki = handle
                .authenticate_keyboard_interactive_start(p.username, None::<String>)
                .await
                .map_err(map_russh_err)?;
            match ki {
                KeyboardInteractiveAuthResponse::Success => {}
                KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                    let answers = vec![p.password.to_string(); prompts.len().max(1)];
                    let ki2 = handle
                        .authenticate_keyboard_interactive_respond(answers)
                        .await
                        .map_err(map_russh_err)?;
                    match ki2 {
                        KeyboardInteractiveAuthResponse::Success => {}
                        KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                            let answers = vec![p.password.to_string(); prompts.len().max(1)];
                            let ki3 = handle
                                .authenticate_keyboard_interactive_respond(answers)
                                .await
                                .map_err(map_russh_err)?;
                            if !matches!(ki3, KeyboardInteractiveAuthResponse::Success) {
                                return Err(AppError::WrongPassword);
                            }
                        }
                        _ => return Err(AppError::WrongPassword),
                    }
                }
                _ => return Err(AppError::WrongPassword),
            }
        }
    }

    let fp = match outcome.lock().unwrap().clone() {
        ServerKeyOutcome::Accepted(fp) => fp,
        _ => {
            return Err(AppError::Ssh {
                detail: "không xác nhận được host key".into(),
            })
        }
    };
    Ok(ConnectedSession { handle, server_fingerprint: fp })
}

// ---------------------------------------------------------------- PTY + bootstrap

pub struct ChannelParts {
    pub read: ChannelReadHalf,
    pub write: ChannelWriteHalf<Msg>,
}

async fn send_line(write: &ChannelWriteHalf<Msg>, line: &str) -> AppResult<()> {
    let mut data = line.as_bytes().to_vec();
    data.push(b'\n');
    write
        .data_bytes(data)
        .await
        .map_err(|e| AppError::Ssh { detail: format!("gửi lệnh thất bại: {e}") })
}

/// Chờ một marker (đầu dòng) hoặc chuỗi lỗi trong output. Timeout 15s/bước.
async fn expect_marker(
    read: &mut ChannelReadHalf,
    marker: &str,
    stage: &str,
    timeout: Duration,
) -> AppResult<Vec<u8>> {
    expect_any_marker(read, &[marker], stage, timeout).await
}

/// Chờ **một trong** các marker (đầu dòng) hoặc chuỗi lỗi trong output —
/// dùng cho lệnh dạng `... && echo OK || echo NO` (chỉ một marker được in).
async fn expect_any_marker(
    read: &mut ChannelReadHalf,
    markers: &[&str],
    stage: &str,
    timeout: Duration,
) -> AppResult<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppError::Timeout { stage: stage.into() });
        }
        match tokio::time::timeout(remaining, read.wait()).await {
            Err(_) => return Err(AppError::Timeout { stage: stage.into() }),
            Ok(None) => return Err(AppError::ConnectionLost),
            Ok(Some(ChannelMsg::Data { data })) => buf.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => buf.extend_from_slice(&data),
            Ok(Some(_)) => {}
        }
        if let Some(err) = classify_error(&buf) {
            return Err(err);
        }
        if markers.iter().any(|m| has_line(&buf, m.as_bytes())) {
            return Ok(buf);
        }
        drain_old(&mut buf);
    }
}

/// Bước sudo: chờ prompt → gửi mật khẩu; NOPASSWD thì qua luôn (mục 5 PLAN).
async fn run_sudo_step(
    read: &mut ChannelReadHalf,
    write: &ChannelWriteHalf<Msg>,
    password: &str,
    marker: &str,
) -> AppResult<()> {
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut buf: Vec<u8> = Vec::new();
    let mut password_sent = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(if password_sent {
                AppError::WrongPassword
            } else {
                AppError::Timeout { stage: "sudo -i".into() }
            });
        }
        match tokio::time::timeout(remaining, read.wait()).await {
            Err(_) => {
                return Err(if password_sent {
                    AppError::WrongPassword
                } else {
                    AppError::Timeout { stage: "sudo -i".into() }
                })
            }
            Ok(None) => return Err(AppError::ConnectionLost),
            Ok(Some(ChannelMsg::Data { data })) => buf.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => buf.extend_from_slice(&data),
            Ok(Some(_)) => {}
        }
        if let Some(err) = classify_error(&buf) {
            return Err(err);
        }
        if has_line(&buf, marker.as_bytes()) {
            return Ok(());
        }
        if looks_like_password_prompt(&buf) {
            if password_sent {
                // hỏi lại sau khi đã gửi → mật khẩu sai
                return Err(AppError::WrongPassword);
            }
            send_line(write, password).await?;
            // sudo flush toàn bộ input TTY trước khi đọc mật khẩu (tgetpass) →
            // dòng `echo {marker}` đã gửi trước đó bị mất; gửi lại để root shell
            // thực thi sau khi auth thành công (NOPASSWD không vào nhánh này).
            send_line(write, &format!("echo {marker}")).await?;
            password_sent = true;
            // dọn buffer để lần quét sau không khớp lại prompt cũ
            buf.clear();
            continue;
        }
        drain_old(&mut buf);
    }
}

/// Bước su - gitlab-runner (chạy với root, không cần mật khẩu).
async fn run_su_step(read: &mut ChannelReadHalf, marker: &str) -> AppResult<()> {
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppError::Timeout { stage: "su - gitlab-runner".into() });
        }
        match tokio::time::timeout(remaining, read.wait()).await {
            Err(_) => return Err(AppError::Timeout { stage: "su - gitlab-runner".into() }),
            Ok(None) => return Err(AppError::ConnectionLost),
            Ok(Some(ChannelMsg::Data { data })) => buf.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => buf.extend_from_slice(&data),
            Ok(Some(_)) => {}
        }
        if let Some(err) = classify_error(&buf) {
            return Err(err);
        }
        if contains(&buf, b"Password:") {
            return Err(AppError::SuFailed {
                detail: "su yêu cầu mật khẩu — phiên hiện tại không phải root".into(),
            });
        }
        if has_line(&buf, marker.as_bytes()) {
            return Ok(());
        }
        drain_old(&mut buf);
    }
}

/// Mở channel mới (PTY 512 cột) + chạy chuỗi khởi tạo: set +H → sudo -i →
/// su - gitlab-runner → kiểm tra kubectl (mục 5 PLAN).
pub async fn open_ready_channel(
    handle: &Handle<ClientHandler>,
    password: &str,
) -> AppResult<ChannelParts> {
    let channel = handle
        .channel_open_session()
        .await
        .map_err(map_russh_err)?;
    channel
        .request_pty(false, "xterm-256color", 512, 24, 0, 0, &[])
        .await
        .map_err(map_russh_err)?;
    channel.request_shell(false).await.map_err(map_russh_err)?;
    let (mut read, write) = channel.split();

    let result = bootstrap(&mut read, &write, password).await;
    if result.is_err() {
        let _ = write.close().await;
    }
    result.map(|_| ChannelParts { read, write })
}

async fn bootstrap(read: &mut ChannelReadHalf, write: &ChannelWriteHalf<Msg>, password: &str) -> AppResult<()> {
    // Bước 1: set +H — tắt history expansion (chống lỗi với `!`)
    let m1 = format!("__LAZY_S1_{}__", crypto::random_suffix(10));
    send_line(write, "set +H").await?;
    send_line(write, &format!("echo {m1}")).await?;
    expect_marker(read, &m1, "khởi tạo shell", STEP_TIMEOUT).await?;

    // Bước 2: sudo -i
    let m2 = format!("__LAZY_S2_{}__", crypto::random_suffix(10));
    send_line(write, "sudo -i").await?;
    send_line(write, &format!("echo {m2}")).await?;
    run_sudo_step(read, write, password, &m2).await?;

    // Bước 3: su - gitlab-runner
    let m3 = format!("__LAZY_S3_{}__", crypto::random_suffix(10));
    send_line(write, "su - gitlab-runner").await?;
    send_line(write, &format!("echo {m3}")).await?;
    run_su_step(read, &m3).await?;

    // Bước 4: kubectl phải tồn tại
    let tag = crypto::random_suffix(10);
    send_line(write, &kubectl::kubectl_check_command(&tag)).await?;
    let ok = format!("__LAZY_S4_OK_{tag}__");
    let no = format!("__LAZY_S4_NO_{tag}__");
    let buf = expect_any_marker(read, &[&ok, &no], "kiểm tra kubectl", STEP_TIMEOUT).await?;
    if has_line(&buf, no.as_bytes()) {
        return Err(AppError::NoKubectl);
    }

    Ok(())
}

/// Chạy `kubectl config use-context` (chỉ trên channel khởi tạo của connect) — mục 6 PLAN.
pub async fn init_use_context(parts: &mut ChannelParts, context: &str) -> AppResult<()> {
    let tag = crypto::random_suffix(10);
    send_line(&parts.write, &kubectl::use_context_check_command(context, &tag)).await?;
    let ok = format!("__LAZY_S5_OK_{tag}__");
    let no = format!("__LAZY_S5_NO_{tag}__");
    let buf = expect_any_marker(&mut parts.read, &[&ok, &no], "chọn context", STEP_TIMEOUT).await?;
    if has_line(&buf, no.as_bytes()) {
        return Err(AppError::ContextNotFound { context: context.into() });
    }
    Ok(())
}

/// Chạy một lệnh + marker trên channel đã sẵn sàng, trả về output (đã cắt tại marker).
async fn run_command(
    read: &mut ChannelReadHalf,
    write: &ChannelWriteHalf<Msg>,
    cmd: &str,
    marker: &str,
    stage: &str,
) -> AppResult<String> {
    send_line(write, cmd).await?;
    let buf = expect_marker(read, marker, stage, STEP_TIMEOUT).await?;
    let text = kubectl::strip_ansi(&String::from_utf8_lossy(&buf));
    Ok(kubectl::cut_at_marker(&text, marker).to_string())
}

/// Wrapper public cho lib.rs (get-contexts, test connection).
pub async fn run_command_for_test(
    parts: &mut ChannelParts,
    cmd: &str,
    marker: &str,
) -> AppResult<String> {
    run_command(&mut parts.read, &parts.write, cmd, marker, "lệnh kubectl").await
}

// ---------------------------------------------------------------- manager

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogOptions {
    pub tail: u32,
    pub timestamps: bool,
    pub since: Option<String>,
}

impl Default for LogOptions {
    fn default() -> Self {
        LogOptions { tail: 500, timestamps: false, since: None }
    }
}

pub struct LogTask {
    pub writer: ChannelWriteHalf<Msg>,
    pub abort: tokio::task::AbortHandle,
}

pub struct ActiveInner {
    pub log_tasks: HashMap<String, LogTask>,
}

pub struct Active {
    pub profile_id: String,
    pub profile_name: String,
    pub context: String,
    pub handle: Handle<ClientHandler>,
    /// Giữ mật khẩu suốt phiên active — mỗi channel mới cần gửi lại cho sudo.
    /// Zeroize khi disconnect / drop (mục 4 PLAN).
    pub password: Zeroizing<String>,
    pub inner: Mutex<ActiveInner>,
}

#[derive(Default)]
pub struct SshManager {
    pub active: Mutex<Option<Arc<Active>>>,
}

impl SshManager {
    pub fn get_active(&self, profile_id: &str) -> AppResult<Arc<Active>> {
        let guard = self.active.lock().unwrap();
        match guard.as_ref() {
            Some(a) if a.profile_id == profile_id => Ok(a.clone()),
            _ => Err(AppError::NotConnected),
        }
    }
}

/// Ngắt phiên: hủy task, đóng channel, disconnect, mật khẩu bị zeroize khi drop.
pub async fn disconnect_active(manager: &SshManager) {
    let taken = manager.active.lock().unwrap().take();
    if let Some(active) = taken {
        let tasks = {
            let mut inner = active.inner.lock().unwrap();
            std::mem::take(&mut inner.log_tasks)
        };
        for (_id, t) in tasks {
            t.abort.abort();
            let _ = t.writer.close().await;
        }
        let _ = active
            .handle
            .disconnect(Disconnect::ByApplication, "user disconnect", "vi")
            .await;
        // active drop → Zeroizing<String> password bị zeroize
    }
}

/// Kết nối tới profile: connect → auth → bootstrap → use-context (mục 2.4, 5, 6 PLAN).
pub async fn connect_profile(
    manager: &SshManager,
    known_hosts_dir: &Path,
    profile: &crate::profile::Profile,
    trust_new_host: bool,
) -> AppResult<String> {
    // nếu đang active profile khác → ngắt trước
    disconnect_active(manager).await;

    let key = format!("{}:{}", profile.host, profile.port);
    let known = load_known_hosts(known_hosts_dir);
    let expected_fp = known.get(&key).cloned();

    let session = connect_and_auth(ConnectParams {
        host: &profile.host,
        port: profile.port,
        username: &profile.username,
        password: &profile.password,
        expected_fp,
        trust_new_host,
    })
    .await?;
    save_known_host(known_hosts_dir, &key, &session.server_fingerprint)?;

    let mut parts = open_ready_channel(&session.handle, &profile.password).await?;
    let init = init_use_context(&mut parts, &profile.context).await;
    if init.is_err() {
        let _ = parts.write.close().await;
        let _ = session.handle.disconnect(Disconnect::ByApplication, "error", "vi").await;
        init?;
    }

    *manager.active.lock().unwrap() = Some(Arc::new(Active {
        profile_id: profile.id.clone(),
        profile_name: profile.name.clone(),
        context: profile.context.clone(),
        handle: session.handle,
        password: Zeroizing::new(profile.password.clone()),
        inner: Mutex::new(ActiveInner {
            log_tasks: HashMap::new(),
        }),
    }));
    Ok(session.server_fingerprint)
}

// ---------------------------------------------------------------- tác vụ

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub request_id: String,
    pub rows: Vec<kubectl::PodRow>,
    pub truncated: bool,
}

/// Tìm pod trên channel riêng; hủy kết quả cũ do frontend xử lý bằng
/// generation counter (mục 7 PLAN) — kết quả stale sẽ bị bỏ qua.
pub async fn search_pods(active: &Active, request_id: &str, keyword: &str) -> AppResult<SearchResult> {
    let mut parts = open_ready_channel(&active.handle, &active.password).await?;
    let cmd = kubectl::search_command(&active.context, keyword, request_id);
    let marker = format!("__LAZY_SEARCH_{request_id}__");
    let result = run_command(&mut parts.read, &parts.write, &cmd, &marker, "tìm pod").await;
    let _ = parts.write.close().await;

    let text = result?;
    let (rows, truncated) = kubectl::parse_pod_rows(&text);
    Ok(SearchResult { request_id: request_id.into(), rows, truncated })
}

/// Lấy danh sách container của pod (jsonpath) — mục 8 PLAN.
pub async fn list_containers(active: &Active, ns: &str, pod: &str) -> AppResult<Vec<String>> {
    let mut parts = open_ready_channel(&active.handle, &active.password).await?;
    let tag = crypto::random_suffix(10);
    let cmd = kubectl::containers_command(pod, ns, &active.context, &tag);
    let marker = format!("__LAZY_CT_{tag}__");
    let res = run_command(&mut parts.read, &parts.write, &cmd, &marker, "lấy danh sách container")
        .await
        .and_then(|text| kubectl::parse_container_names(&text, pod, ns));
    let _ = parts.write.close().await;
    res
}

fn detect_log_start_error(buf: &[u8], ns: &str, pod: &str, container: &str) -> Option<AppError> {
    let hay = String::from_utf8_lossy(buf);
    // chỉ khớp định dạng lỗi của kubectl — nội dung log của app có thể chứa
    // "not found" chung chung (vd. "config file not found") → không được nhầm.
    if hay.contains("Error from server (NotFound)")
        || hay.contains(&format!("pods \"{pod}\" not found"))
    {
        return Some(AppError::PodNotFound { pod: pod.into(), ns: ns.into() });
    }
    if hay.contains(&format!("container {container} not found"))
        || hay.contains("failed to get container")
    {
        return Some(AppError::Other {
            detail: format!("Container '{container}' không tồn tại trong pod '{ns}/{pod}'"),
        });
    }
    if hay.contains("unable to retrieve container logs") {
        return Some(AppError::Other {
            detail: "Không lấy được log của container này".into(),
        });
    }
    None
}

fn bytes_to_lines(buf: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, &b) in buf.iter().enumerate() {
        if b == b'\n' {
            let mut end = i;
            if end > start && buf[end - 1] == b'\r' {
                end -= 1;
            }
            out.push(String::from_utf8_lossy(&buf[start..end]).to_string());
            start = i + 1;
        }
    }
    if start < buf.len() {
        out.push(String::from_utf8_lossy(&buf[start..]).to_string());
    }
    out
}

/// Mở stream log trên channel riêng; phát hiện lỗi khởi động trong 3s rồi
/// chuyển sang chế độ stream, batch 100ms emit event `log:<tabId>` — mục 8 PLAN.
pub async fn start_log_stream(
    app: tauri::AppHandle,
    manager: &SshManager,
    profile_id: &str,
    tab_id: &str,
    ns: &str,
    pod: &str,
    container: &str,
    options: LogOptions,
) -> AppResult<()> {
    let active = manager.get_active(profile_id)?;

    // hủy stream cũ của tab nếu có
    let old = {
        let mut inner = active.inner.lock().unwrap();
        inner.log_tasks.remove(tab_id)
    };
    if let Some(t) = old {
        t.abort.abort();
        let _ = t.writer.close().await;
    }

    let mut parts = open_ready_channel(&active.handle, &active.password).await?;
    let cmd = kubectl::log_command(
        ns,
        pod,
        container,
        options.tail.max(1),
        options.timestamps,
        options.since.as_deref(),
        &active.context,
    );
    send_line(&parts.write, &cmd).await?;

    // chờ tối đa 3s để phát hiện lỗi khởi động
    let mut buf: Vec<u8> = Vec::new();
    let deadline = Instant::now() + LOG_START_DETECT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, parts.read.wait()).await {
            Err(_) => break, // hết 3s không lỗi → stream bình thường
            Ok(None) => {
                let _ = parts.write.close().await;
                if let Some(e) = detect_log_start_error(&buf, ns, pod, container) {
                    return Err(e);
                }
                return Err(AppError::ConnectionLost);
            }
            Ok(Some(ChannelMsg::Data { data })) => buf.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => buf.extend_from_slice(&data),
            Ok(Some(ChannelMsg::Eof)) | Ok(Some(ChannelMsg::Close)) => {
                let _ = parts.write.close().await;
                if let Some(e) = detect_log_start_error(&buf, ns, pod, container) {
                    return Err(e);
                }
                return Err(AppError::Other {
                    detail: "Stream kết thúc ngay sau khi mở".into(),
                });
            }
            Ok(Some(_)) => {}
        }
        if let Some(e) = detect_log_start_error(&buf, ns, pod, container) {
            let _ = parts.write.close().await;
            return Err(e);
        }
    }

    // emit phần buffer 3s đầu (chỉ dòng hoàn chỉnh; phần dư cuối chuyển sang
    // stream_loop làm carry để không cắt đôi một dòng)
    let ev = format!("log:{tab_id}");
    let mut carry: Vec<u8> = Vec::new();
    if !buf.is_empty() {
        match buf.iter().rposition(|&b| b == b'\n') {
            Some(pos) => {
                carry = buf[pos + 1..].to_vec();
                let lines = bytes_to_lines(&buf[..=pos]);
                let _ = app.emit(&ev, serde_json::json!({ "lines": lines }));
            }
            None => carry = buf, // chưa có dòng hoàn chỉnh → chuyển hết sang stream_loop
        }
    }

    let (read, writer) = (parts.read, parts.write);
    let task_app = app.clone();
    let task_profile = active.profile_id.clone();
    let task_tab = tab_id.to_string();
    let join = tokio::spawn(async move {
        stream_loop(task_app, task_profile, task_tab, read, carry).await;
    });

    active.inner.lock().unwrap().log_tasks.insert(
        tab_id.to_string(),
        LogTask { writer, abort: join.abort_handle() },
    );
    Ok(())
}

fn flush_batch(app: &tauri::AppHandle, ev: &str, batch: &mut Vec<String>) {
    if !batch.is_empty() {
        let lines = std::mem::take(batch);
        let _ = app.emit(ev, serde_json::json!({ "lines": lines }));
    }
}

/// Vòng lặp stream: gom dòng, batch 100ms emit; kênh đóng → emit `logend:<tabId>`.
/// `initial_carry` là phần dư chưa kết thúc dòng từ buffer 3s đầu.
async fn stream_loop(
    app: tauri::AppHandle,
    _profile_id: String,
    tab_id: String,
    mut read: ChannelReadHalf,
    mut carry: Vec<u8>,
) {
    let ev = format!("log:{tab_id}");
    let ev_end = format!("logend:{tab_id}");
    let mut batch: Vec<String> = Vec::new();
    let mut last_flush = Instant::now();
    loop {
        let got: Option<ChannelMsg> = if batch.is_empty() && carry.is_empty() {
            read.wait().await
        } else {
            match tokio::time::timeout(STREAM_FLUSH, read.wait()).await {
                Ok(m) => m,
                Err(_) => {
                    if last_flush.elapsed() >= STREAM_FLUSH {
                        flush_batch(&app, &ev, &mut batch);
                        last_flush = Instant::now();
                    }
                    continue;
                }
            }
        };
        match got {
            None => {
                if !carry.is_empty() {
                    batch.push(String::from_utf8_lossy(&carry).to_string());
                    carry.clear();
                }
                flush_batch(&app, &ev, &mut batch);
                let _ = app.emit(&ev_end, serde_json::json!({}));
                break;
            }
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                carry.extend_from_slice(&data);
                while let Some(pos) = carry.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = carry.drain(..=pos).collect();
                    let mut end = line.len() - 1; // bỏ '\n'
                    if end > 0 && line[end - 1] == b'\r' {
                        end -= 1; // bỏ '\r'
                    }
                    batch.push(String::from_utf8_lossy(&line[..end]).to_string());
                }
                if last_flush.elapsed() >= STREAM_FLUSH {
                    flush_batch(&app, &ev, &mut batch);
                    last_flush = Instant::now();
                }
            }
            Some(_) => {}
        }
    }
}

/// Đóng tab log → đóng channel + hủy task (mục 8 PLAN).
pub async fn close_log_tab(active: &Active, tab_id: &str) {
    let t = {
        let mut inner = active.inner.lock().unwrap();
        inner.log_tasks.remove(tab_id)
    };
    if let Some(t) = t {
        t.abort.abort();
        let _ = t.writer.close().await;
    }
}

// ---------------------------------------------------------------- known_hosts

pub fn load_known_hosts(dir: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(dir.join("known_hosts.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_known_host(dir: &Path, key: &str, fp: &str) -> AppResult<()> {
    let mut map = load_known_hosts(dir);
    map.insert(key.to_string(), fp.to_string());
    let data = serde_json::to_string_pretty(&map)
        .map_err(|e| AppError::Io { detail: e.to_string() })?;
    // ghi file tạm + rename để crash giữa chừng không làm hỏng file
    let path = dir.join("known_hosts.json");
    let tmp = dir.join("known_hosts.json.tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_line_start_only() {
        // dòng echo lệnh chứa marker nhưng KHÔNG ở đầu dòng → không khớp
        let echo_only = b"echo __LAZY_S1_abc123__\r\n";
        assert!(!has_line(echo_only, b"__LAZY_S1_abc123__"));
        // output thực thi ở đầu dòng → khớp
        let with_output = b"echo __LAZY_S1_abc123__\r\n__LAZY_S1_abc123__\r\n";
        assert!(has_line(with_output, b"__LAZY_S1_abc123__"));
        // marker ở đầu buffer
        assert!(has_line(b"__LAZY_S1_abc123__\r\n", b"__LAZY_S1_abc123__"));
        assert!(!has_line(b"prefix __LAZY_S1_abc123__\r\n", b"__LAZY_S1_abc123__"));
    }

    #[test]
    fn sudo_error_classification() {
        assert!(matches!(
            classify_error(b"sudo: 3 incorrect password attempts\r\n"),
            Some(AppError::WrongPassword)
        ));
        assert!(matches!(
            classify_error(b"Sorry, try again.\r\n"),
            Some(AppError::WrongPassword)
        ));
        assert!(matches!(
            classify_error(b"user is not in the sudoers file.\r\n"),
            Some(AppError::SudoFailed { .. })
        ));
        assert!(matches!(
            classify_error(b"su: Authentication failure\r\n"),
            Some(AppError::SuFailed { .. })
        ));
        assert!(classify_error(b"total 0\ndrwxr-xr-x 1 root root\r\n").is_none());
    }

    #[test]
    fn password_prompt_detection() {
        assert!(looks_like_password_prompt(b"[sudo] password for abc: "));
        assert!(looks_like_password_prompt(b"Password: "));
        assert!(!looks_like_password_prompt(b"root@host:~# "));
    }

    #[test]
    fn log_start_error_detection() {
        assert!(matches!(
            detect_log_start_error(b"Error from server (NotFound): pods \"x\" not found", "ns", "x", "c"),
            Some(AppError::PodNotFound { .. })
        ));
        assert!(detect_log_start_error(b"2026-11-20 log line one\r\n", "ns", "x", "c").is_none());
        assert!(detect_log_start_error(b"container app not found in pod", "ns", "x", "app").is_some());
        // nội dung log của app chứa "not found" → không được nhầm PodNotFound
        assert!(detect_log_start_error(b"config file not found, using defaults", "ns", "x", "c").is_none());
        // lỗi kubectl không có tiền tố "Error from server" vẫn khớp theo pod
        assert!(matches!(
            detect_log_start_error(b"pods \"x\" not found", "ns", "x", "c"),
            Some(AppError::PodNotFound { .. })
        ));
        // lỗi của pod khác → không khớp
        assert!(detect_log_start_error(b"pods \"other\" not found", "ns", "x", "c").is_none());
    }

    #[test]
    fn bytes_to_lines_works() {
        let lines = bytes_to_lines(b"one\r\ntwo\npartial");
        assert_eq!(lines, vec!["one", "two", "partial"]);
    }
}
