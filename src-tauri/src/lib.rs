mod crypto;
mod error;
mod kubectl;
mod profile;
mod ssh;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Emitter, Manager};
use zeroize::Zeroizing;

use crate::error::{AppError, AppResult};
use crate::profile::{Profile, ProfileInput};
use crate::ssh::{LogOptions, SshManager};

/// Trạng thái vault (đã unlock): key + kdf + danh sách profile trong bộ nhớ.
pub struct Vault {
    pub key: Zeroizing<[u8; 32]>,
    pub kdf: u8,
    pub salt: Option<[u8; 16]>,
    pub profiles: Vec<Profile>,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub vault: Mutex<Option<Vault>>,
    pub ssh: SshManager,
}

impl AppState {
    fn profiles_path(&self) -> PathBuf {
        self.data_dir.join("profiles.dat")
    }

    /// Chưa có file dữ liệu → khởi tạo vault mới với key từ OS keyring (lần chạy đầu).
    fn ensure_vault(&self) -> AppResult<()> {
        let mut guard = self.vault.lock().unwrap();
        if guard.is_none() {
            if self.profiles_path().exists() {
                return Err(AppError::Other {
                    detail: "Kho dữ liệu đang khóa — hãy mở khóa trước".into(),
                });
            }
            let key = crypto::get_or_create_keyring_key()?;
            *guard = Some(Vault {
                key,
                kdf: crypto::KDF_KEYRING,
                salt: None,
                profiles: Vec::new(),
            });
        }
        Ok(())
    }

    pub fn persist(&self, vault: &Vault) -> AppResult<()> {
        let plaintext = serde_json::to_vec(&vault.profiles)
            .map_err(|e| AppError::Crypto { detail: e.to_string() })?;
        let bytes = crypto::encrypt_to_bytes(&vault.key, vault.kdf, vault.salt.as_ref(), &plaintext)?;
        atomic_write(&self.profiles_path(), &bytes)?;
        Ok(())
    }

    pub fn with_vault<T>(&self, f: impl FnOnce(&Vault) -> AppResult<T>) -> AppResult<T> {
        let guard = self.vault.lock().unwrap();
        match guard.as_ref() {
            Some(v) => f(v),
            None => Err(AppError::Other { detail: "Kho dữ liệu chưa được mở khóa".into() }),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStatus {
    /// đã có file dữ liệu (profiles.dat tồn tại)
    pub initialized: bool,
    /// None khi chưa có file
    pub kdf: Option<u8>,
    /// đang khóa (cần passphrase với kdf=1)
    pub locked: bool,
    pub need_passphrase: bool,
}

/// Ghi file tạm + rename — crash giữa chừng không làm hỏng file đích.
fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> AppResult<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn read_vault_from_disk(data_dir: &std::path::Path) -> AppResult<Option<Vault>> {
    let path = data_dir.join("profiles.dat");
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read(&path)?;
    let header = crypto::parse_header(&data)?;
    Ok(Some(Vault {
        key: Zeroizing::new([0u8; 32]),
        kdf: header.kdf,
        salt: header.salt,
        profiles: Vec::new(),
    }))
}

fn decrypt_vault(data: &[u8], key: &[u8; 32]) -> AppResult<Vec<Profile>> {
    let plaintext = crypto::decrypt_from_bytes(data, key)?;
    serde_json::from_slice(&plaintext).map_err(|e| AppError::Crypto { detail: e.to_string() })
}

// ---------------------------------------------------------------- store commands

#[tauri::command]
async fn store_status(state: tauri::State<'_, AppState>) -> Result<StoreStatus, AppError> {
    let on_disk = read_vault_from_disk(&state.data_dir)?;
    let vault = state.vault.lock().unwrap();
    let initialized = vault.is_some() || on_disk.is_some();
    Ok(match on_disk {
        None => StoreStatus { initialized: false, kdf: None, locked: false, need_passphrase: false },
        Some(disk) => {
            let locked = vault.is_none();
            StoreStatus {
                initialized,
                kdf: Some(disk.kdf),
                locked,
                need_passphrase: locked && disk.kdf == crypto::KDF_ARGON2ID,
            }
        }
    })
}

/// Mở khóa khi mở app (mục 3.4 PLAN): kdf=0 → keyring; kdf=1 → passphrase.
#[tauri::command]
async fn store_unlock(
    state: tauri::State<'_, AppState>,
    passphrase: Option<String>,
) -> Result<Vec<Profile>, AppError> {
    let data = std::fs::read(state.profiles_path())?;
    let header = crypto::parse_header(&data)?;
    let key = match header.kdf {
        crypto::KDF_KEYRING => crypto::get_or_create_keyring_key()?,
        crypto::KDF_ARGON2ID => {
            let pass = passphrase.ok_or(AppError::UnlockFailed)?;
            crypto::derive_argon2id(&pass, header.salt.as_ref().unwrap())?
        }
        other => {
            return Err(AppError::Crypto {
                detail: format!("KDF không xác định: {other}"),
            })
        }
    };
    let profiles = decrypt_vault(&data, &key)?;
    *state.vault.lock().unwrap() = Some(Vault {
        key,
        kdf: header.kdf,
        salt: header.salt,
        profiles: profiles.clone(),
    });
    Ok(profiles)
}

/// Đổi key mã hóa: None → keyring (key random mới); Some(p) → Argon2id passphrase (mục 4 PLAN).
#[tauri::command]
async fn store_change_key(
    state: tauri::State<'_, AppState>,
    new_passphrase: Option<String>,
) -> Result<(), AppError> {
    let mut guard = state.vault.lock().unwrap();
    let vault = guard
        .as_mut()
        .ok_or_else(|| AppError::Other { detail: "Kho dữ liệu chưa được mở khóa".into() })?;

    let (new_key, new_kdf, new_salt) = match &new_passphrase {
        None => {
            let key = crypto::random_key();
            crypto::set_keyring_key(&key)?;
            (key, crypto::KDF_KEYRING, None)
        }
        Some(pass) => {
            if pass.is_empty() {
                return Err(AppError::Other { detail: "Passphrase không được để trống".into() });
            }
            let salt = crypto::random_salt();
            (crypto::derive_argon2id(pass, &salt)?, crypto::KDF_ARGON2ID, Some(salt))
        }
    };
    // giải mã toàn bộ bằng key cũ (đã trong bộ nhớ) → mã hóa lại
    let plaintext = serde_json::to_vec(&vault.profiles)
        .map_err(|e| AppError::Crypto { detail: e.to_string() })?;
    let bytes = crypto::encrypt_to_bytes(&new_key, new_kdf, new_salt.as_ref(), &plaintext)?;
    atomic_write(&state.profiles_path(), &bytes)?;
    if new_kdf == crypto::KDF_ARGON2ID {
        let _ = crypto::delete_keyring_key();
    }
    vault.key = new_key;
    vault.kdf = new_kdf;
    vault.salt = new_salt;
    Ok(())
}

// ---------------------------------------------------------------- profile commands

#[tauri::command]
async fn profile_list(state: tauri::State<'_, AppState>) -> Result<Vec<Profile>, AppError> {
    state.ensure_vault()?;
    state.with_vault(|v| Ok(v.profiles.clone()))
}

#[tauri::command]
async fn profile_save(
    state: tauri::State<'_, AppState>,
    profile: ProfileInput,
) -> Result<Vec<Profile>, AppError> {
    state.ensure_vault()?;
    let mut guard = state.vault.lock().unwrap();
    let vault = guard
        .as_mut()
        .ok_or_else(|| AppError::Other { detail: "Kho dữ liệu chưa được mở khóa".into() })?;
    profile::upsert(&mut vault.profiles, profile)?;
    state.persist(vault)?;
    Ok(vault.profiles.clone())
}

#[tauri::command]
async fn profile_delete(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Vec<Profile>, AppError> {
    state.ensure_vault()?;
    let mut guard = state.vault.lock().unwrap();
    let vault = guard
        .as_mut()
        .ok_or_else(|| AppError::Other { detail: "Kho dữ liệu chưa được mở khóa".into() })?;
    profile::delete(&mut vault.profiles, &id)?;
    state.persist(vault)?;
    Ok(vault.profiles.clone())
}

#[tauri::command]
async fn profile_duplicate(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Vec<Profile>, AppError> {
    state.ensure_vault()?;
    let mut guard = state.vault.lock().unwrap();
    let vault = guard
        .as_mut()
        .ok_or_else(|| AppError::Other { detail: "Kho dữ liệu chưa được mở khóa".into() })?;
    profile::duplicate(&mut vault.profiles, &id)?;
    state.persist(vault)?;
    Ok(vault.profiles.clone())
}

// ---------------------------------------------------------------- connection commands

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub fingerprint: String,
    pub contexts: Vec<String>,
}

/// Test kết nối từ form (không cần lưu): auth → TOFU → chuỗi sudo/su → kubectl → contexts (mục 2.2 PLAN).
#[tauri::command]
async fn test_connection(
    state: tauri::State<'_, AppState>,
    host: String,
    port: u16,
    username: String,
    password: String,
    trust_new_host: bool,
) -> Result<TestResult, AppError> {
    let (input_user, host_only, input_port) = profile::parse_host_input(&host);
    let username = match (&username, &input_user) {
        (u, _) if !u.trim().is_empty() => u.trim().to_string(),
        (_, Some(um)) => um.clone(),
        _ => return Err(AppError::Other { detail: "Thiếu username".into() }),
    };
    let port = port.max(1).min(65535);
    let port = if port != 22 { port } else { input_port.unwrap_or(port) };
    let host = host_only;

    let key = format!("{host}:{port}");
    let known = ssh::load_known_hosts(&state.data_dir);
    let expected_fp = known.get(&key).cloned();

    let session = ssh::connect_and_auth(ssh::ConnectParams {
        host: &host,
        port,
        username: &username,
        password: &password,
        expected_fp,
        trust_new_host,
    })
    .await?;

    let result = async {
        let mut parts = ssh::open_ready_channel(&session.handle, &password).await?;
        let tag = crypto::random_suffix(10);
        let cmd = kubectl::get_contexts_command(&tag);
        let marker = format!("__LAZY_CTX_{tag}__");
        let res = ssh::run_command_for_test(&mut parts, &cmd, &marker)
            .await
            .map(|text| kubectl::parse_contexts(&text));
        let _ = parts.write.close().await;
        res
    }
    .await;

    ssh::save_known_host(&state.data_dir, &key, &session.server_fingerprint)?;
    let _ = session
        .handle
        .disconnect(russh::Disconnect::ByApplication, "test done", "vi")
        .await;

    match result {
        Ok(contexts) => Ok(TestResult {
            fingerprint: session.server_fingerprint,
            contexts,
        }),
        Err(e) => Err(e),
    }
}

/// Kết nối theo profile — phát event `conn:<profileId>` (mục 2.4, 9 PLAN).
#[tauri::command]
async fn connect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    profile_id: String,
    trust_new_host: bool,
) -> Result<(), AppError> {
    let profile = state.with_vault(|v| {
        v.profiles
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
            .ok_or(AppError::ProfileNotFound)
    })?;

    let event = format!("conn:{profile_id}");
    let emit = |payload: serde_json::Value| {
        let _ = app.emit(&event, payload);
    };
    emit(serde_json::json!({ "state": "connecting", "message": null }));

    match ssh::connect_profile(&state.ssh, &state.data_dir, &profile, trust_new_host).await {
        Ok(_fp) => {
            emit(serde_json::json!({ "state": "ready", "message": null }));
            Ok(())
        }
        Err(e) => {
            emit(serde_json::json!({ "state": "error", "message": e.to_string() }));
            Err(e)
        }
    }
}

/// Ngắt kết nối profile đang active (mục 2.3 PLAN).
#[tauri::command]
async fn disconnect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), AppError> {
    let profile_id = {
        let guard = state.ssh.active.lock().unwrap();
        guard.as_ref().map(|a| a.profile_id.clone())
    };
    ssh::disconnect_active(&state.ssh).await;
    if let Some(pid) = profile_id {
        let _ = app.emit(
            &format!("conn:{pid}"),
            serde_json::json!({ "state": "disconnected", "message": null }),
        );
    }
    Ok(())
}

/// Danh sách context từ phiên đang kết nối (mục 6 PLAN).
#[tauri::command]
async fn get_contexts(state: tauri::State<'_, AppState>) -> Result<Vec<String>, AppError> {
    let active = state
        .ssh
        .active
        .lock()
        .unwrap()
        .as_ref()
        .map(|a| a.profile_id.clone());
    let profile_id = active.ok_or(AppError::NotConnected)?;
    let active = state.ssh.get_active(&profile_id)?;
    let mut parts = ssh::open_ready_channel(&active.handle, &active.password).await?;
    let tag = crypto::random_suffix(10);
    let cmd = kubectl::get_contexts_command(&tag);
    let marker = format!("__LAZY_CTX_{tag}__");
    let res = ssh::run_command_for_test(&mut parts, &cmd, &marker)
        .await
        .map(|text| kubectl::parse_contexts(&text));
    let _ = parts.write.close().await;
    res
}

// ---------------------------------------------------------------- pod + log commands

/// Tìm pod — debounce + hủy request cũ do frontend lo (mục 7 PLAN).
#[tauri::command]
async fn search_pods(
    state: tauri::State<'_, AppState>,
    request_id: String,
    keyword: String,
) -> Result<ssh::SearchResult, AppError> {
    let profile_id = {
        let guard = state.ssh.active.lock().unwrap();
        guard.as_ref().map(|a| a.profile_id.clone())
    };
    let profile_id = profile_id.ok_or(AppError::NotConnected)?;
    let active = state.ssh.get_active(&profile_id)?;
    if keyword.trim().len() < 3 {
        return Ok(ssh::SearchResult {
            request_id,
            rows: vec![],
            truncated: false,
        });
    }
    ssh::search_pods(&active, &request_id, keyword.trim()).await
}

#[tauri::command]
async fn open_log_tab(
    state: tauri::State<'_, AppState>,
    tab_id: String,
    ns: String,
    pod: String,
) -> Result<Vec<String>, AppError> {
    let _ = tab_id;
    let profile_id = {
        let guard = state.ssh.active.lock().unwrap();
        guard.as_ref().map(|a| a.profile_id.clone())
    };
    let profile_id = profile_id.ok_or(AppError::NotConnected)?;
    let active = state.ssh.get_active(&profile_id)?;
    ssh::list_containers(&active, &ns, &pod).await
}

/// Bắt đầu stream log cho tab — event `log:<tabId>` / `logend:<tabId>` (mục 8 PLAN).
#[tauri::command]
async fn start_log_stream(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    tab_id: String,
    ns: String,
    pod: String,
    container: String,
    options: LogOptions,
) -> Result<(), AppError> {
    let profile_id = {
        let guard = state.ssh.active.lock().unwrap();
        guard.as_ref().map(|a| a.profile_id.clone())
    };
    let profile_id = profile_id.ok_or(AppError::NotConnected)?;
    ssh::start_log_stream(app, &state.ssh, &profile_id, &tab_id, &ns, &pod, &container, options)
        .await
}

#[tauri::command]
async fn close_log_tab(
    state: tauri::State<'_, AppState>,
    tab_id: String,
) -> Result<(), AppError> {
    let profile_id = {
        let guard = state.ssh.active.lock().unwrap();
        guard.as_ref().map(|a| a.profile_id.clone())
    };
    if let Some(profile_id) = profile_id {
        if let Ok(active) = state.ssh.get_active(&profile_id) {
            ssh::close_log_tab(&active, &tab_id).await;
        }
    }
    Ok(())
}

/// Lưu log ra file (frontend đã chọn đường dẫn qua dialog plugin) — mục 8 PLAN.
#[tauri::command]
async fn save_log_file(path: String, content: String) -> Result<(), AppError> {
    if path.trim().is_empty() {
        return Err(AppError::Other { detail: "Đường dẫn trống".into() });
    }
    std::fs::write(&path, content)?;
    Ok(())
}

// ---------------------------------------------------------------- run

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("không xác định được thư mục dữ liệu: {e}"))?;
            std::fs::create_dir_all(&data_dir)?;

            // kdf=0 → tự mở khóa bằng keyring ngay khi khởi động (mục 3.4 PLAN)
            let vault = match read_vault_from_disk(&data_dir) {
                Err(e) => {
                    eprintln!("Lỗi đọc kho dữ liệu: {e}");
                    None
                }
                Ok(Some(disk)) if disk.kdf == crypto::KDF_KEYRING => {
                    match std::fs::read(data_dir.join("profiles.dat"))
                        .ok()
                        .and_then(|d| {
                            crypto::get_or_create_keyring_key()
                                .ok()
                                .and_then(|key| decrypt_vault(&d, &key).ok().map(|p| (key, p)))
                        }) {
                        Some((key, profiles)) => Some(Vault {
                            key,
                            kdf: crypto::KDF_KEYRING,
                            salt: None,
                            profiles,
                        }),
                        None => None, // keyring hỏng → hiện màn unlock
                    }
                }
                Ok(_) => None, // kdf=1 → chờ passphrase từ UI
            };

            app.manage(AppState {
                data_dir,
                vault: Mutex::new(vault),
                ssh: SshManager::default(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            store_status,
            store_unlock,
            store_change_key,
            profile_list,
            profile_save,
            profile_delete,
            profile_duplicate,
            test_connection,
            connect,
            disconnect,
            get_contexts,
            search_pods,
            open_log_tab,
            start_log_stream,
            close_log_tab,
            save_log_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
