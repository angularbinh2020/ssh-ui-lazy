# PLAN — SSH UI Lazy (bản 2.0 — chờ confirm)

Ứng dụng desktop **Tauri 2 (Windows)** dùng để: đăng nhập SSH bằng mật khẩu, vào môi trường `root` → `gitlab-runner`, chọn kubectl context, tìm kiếm pod và xem log realtime.

## 0. Thay đổi so với bản trước
- Chốt wireframe + layout; Screen 1 chỉ hiện lần đầu, sau đó là modal quản lý profile
- Bổ sung schema profile, format file mã hóa, vị trí lưu, luồng unlock khi mở app
- Cơ chế marker chi tiết cho `sudo -i` / `su -` (xử lý cả NOPASSWD, hỏi mật khẩu, mật khẩu sai)
- `set +H` chống history expansion; PTY 512 cột chống wrap làm hỏng parse
- Giới hạn kết quả tìm pod (`head -n 200`) + hủy request cũ
- Cách xác định container của pod (jsonpath); hành vi khi pod chết/stream kết thúc
- Keepalive 30s, timeout cụ thể, batch stream 100ms
- **Sửa**: mật khẩu KHÔNG xóa ngay sau khi dùng (mỗi channel mới cần gửi lại cho sudo) — chỉ xóa khi ngắt phiên
- Stack frontend cụ thể, cấu trúc thư mục, danh sách command/event
- Milestones, kế hoạch test, build NSIS/MSI, mục ngoài phạm vi

## 1. Phạm vi

### 1.1 Trong phạm vi
- Quản lý profile (thêm/sửa/xóa/nhân bản/test kết nối), mã hóa cục bộ
- SSH bằng mật khẩu, chạy chuỗi `sudo -i` → `su - gitlab-runner` → chọn context
- Tìm pod, xem log realtime nhiều tab

### 1.2 Ngoài phạm vi (non-goals)
- SSH key auth, 2FA/OTP, bastion/jump host, agent forwarding
- Chạy nhiều profile active cùng lúc (chỉ 1 profile active)
- Code signing installer, auto-update
- Quản lý/sửa kubeconfig, chạy lệnh kubectl tùy ý ngoài các tính năng trên

## 2. Màn hình & UX

### 2.1 Wireframe màn hình chính
```
┌────────────────────────────────────────────────────────┐
│ [Profile ▼]  ● Sẵn sàng   [⟳ Kết nối lại] [☀/🌙] [⚙] │
├────────────────────────────────────────────────────────┤
│ Tìm pod: [______________]                    [Refresh] │
│ ┌────────────────────────────────────────────────────┐ │
│ │ NS │ Pod │ Ready │ Status │ Restarts │ Age         │ │
│ └────────────────────────────────────────────────────┘ │
├────────────────────────────────────────────────────────┤
│ [ns/pod-abc ×] [ns/pod-xyz ×]                          │
│ ┌────────────────────────────────────────────────────┐ │
│ │ [⏸ auto-scroll] [🔍 tìm] [🧹 xóa] [💾 lưu] [⚙ opts]│ │
│ │ log viewer (monospace, ANSI màu)                   │ │
│ └────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────┘
```

### 2.2 Màn hình Profile
- Chỉ hiển thị **khi chưa có profile nào**. Sau đó mọi thao tác profile là modal từ màn hình chính.
- Form: **Tên profile*** (duy nhất), **SSH host*** (`host` hoặc `user@host`), **Port** (mặc định 22, nhận cả `user@host:port`), **Mật khẩu***, **Context*** (chọn từ danh sách lấy qua Test kết nối, vẫn cho nhập tay).
- Nút **Test kết nối** (không cần lưu trước): auth SSH → xác nhận fingerprint → chuỗi sudo/su → kiểm tra kubectl → trả danh sách contexts.
- Danh sách profile: Thêm / Sửa / Xóa (có confirm) / Nhân bản.

### 2.3 Màn hình chính
- Thanh trên: select profile, trạng thái kết nối, nút Kết nối lại, nút toggle dark/light, nút Quản lý profile.
- Theme: có **toggle dark/light**, lưu lựa chọn vào localStorage.
- Khu tìm pod + bảng kết quả; khu tab log phía dưới.
- Đổi profile → confirm trước khi đóng các tab đang mở.
- Đóng app khi đang kết nối → ngắt kết nối, không cần confirm.

### 2.4 Trạng thái kết nối
`Mất kết nối → Đang kết nối (TCP + auth + chuỗi sudo/su + kubectl) → Sẵn sàng / Lỗi (kèm message tiếng Việt)` + nút Kết nối lại.

## 3. Mô hình dữ liệu

### 3.1 Profile
```json
{
  "id": "uuid",
  "name": "staging-126",
  "host": "10.110.30.126",
  "port": 22,
  "username": "abc",
  "password": "…",
  "context": "staging-cluster",
  "createdAt": "2026-11-20T00:00:00Z",
  "updatedAt": "2026-11-20T00:00:00Z"
}
```

### 3.2 File mã hóa
- Đường dẫn: `%APPDATA%\ssh-ui-lazy\profiles.dat` (Tauri `app_data_dir`).
- Format: `magic "SSHLZY" | version u8 | kdf u8 (0 = keyring, 1 = argon2id) | [salt 16B nếu kdf=1] | nonce 12B | ciphertext`
- Plaintext: JSON mảng profile. Thay đổi cấu trúc → tăng `version`, có bước migrate.
- Header (magic + version + kdf + salt) dùng làm **AAD** cho AES-GCM.

### 3.3 known_hosts
- `%APPDATA%\ssh-ui-lazy\known_hosts.json` — `{"host:port": "SHA256:fingerprint"}` (không phải dữ liệu mật, lưu plain).

### 3.4 Unlock khi mở app
- `kdf=0`: đọc key 32B từ OS keyring (service `ssh-ui-lazy`, account `master-key`) → mở thẳng.
- `kdf=1`: hiện modal nhập passphrase → Argon2id derive → sai thì báo lỗi, nhập lại.

## 4. Bảo mật
- AES-256-GCM, nonce 12B ngẫu nhiên mỗi lần ghi, lưu kèm ciphertext, AAD = header.
- Key mặc định: 32B random lưu **OS keyring** (Windows Credential Manager) — không lưu cạnh file data.
- User ghi đè key bằng passphrase: derive bằng **Argon2id** (m=64MiB, t=3, p=4, salt 16B random). Đổi key = giải mã bằng key cũ → mã hóa lại toàn bộ → ghi file. Cảnh báo: quên key = mất dữ liệu.
- Fingerprint host kiểu **TOFU**: lần đầu hiện SHA256 cho user xác nhận rồi lưu; lệch → chặn + cảnh báo.
- Mật khẩu: giữ trong bộ nhớ (buffer zeroized) trong suốt phiên active — vì mỗi channel mới cần gửi lại cho `sudo`; xóa khi disconnect / đổi profile / đóng app. Không log, không persist ngoài file mã hóa.
- Log của app không chứa mật khẩu.

## 5. Kết nối SSH (russh)
- Auth: thử `password` trước; server chỉ hỗ trợ `keyboard-interactive` → trả password qua cơ chế đó. Sai mật khẩu → lỗi riêng, rõ ràng.
- Timeout TCP + auth: 10s. Keepalive 30s, fail 3 lần liên tiếp → trạng thái Mất kết nối (tab log báo lỗi + nút kết nối lại).
- Chuỗi khởi tạo trên channel có **PTY** (`xterm-256color`, 512 cột để tránh wrap làm hỏng parse):
  1. `set +H` — tắt history expansion của bash interactive (chống lỗi với `!` trong từ khóa tìm kiếm)
  2. `sudo -i` — nếu output là prompt `[sudo] password for …` / `Password:` thì gửi mật khẩu; NOPASSWD thì qua luôn
  3. `su - gitlab-runner` (chạy với root, không cần mật khẩu)
  4. `command -v kubectl` — không có → lỗi "Không có kubectl trên máy chủ"
- Xác nhận từng bước bằng **marker**: gửi dòng `echo __LAZY_S<n>_<random>__` (buffer qua PTY, shell kế tiếp sẽ đọc), chờ marker trong output; đồng thời phát hiện chuỗi lỗi (`incorrect password attempts`…). Timeout 15s/bước → báo lỗi phân loại.
- Mỗi tác vụ (search, mỗi tab log) dùng **channel SSH riêng trên cùng connection** và tự chạy lại chuỗi 2–4 (không chạy lại `use-context`).
- Phân loại lỗi: sai mật khẩu, host không đạt/từ chối, timeout, fingerprint thay đổi, không có kubectl, context không tồn tại, mất kết nối giữa chừng.

## 6. kubectl & context
- Sau khi sẵn sàng chạy `kubectl config use-context '<ctx>'` (đúng yêu cầu); ngoài ra **mọi** lệnh kubectl đều kèm `--context '<ctx>'` → 2 phiên chạy song song không ghi đè kubeconfig của nhau.
- Lấy danh sách context cho form: `kubectl config get-contexts -o name`.

## 7. Tìm pod
- Kích hoạt khi nhập ≥3 ký tự, debounce 400ms, hủy kết quả cũ (generation counter — kết quả về sau khi hủy thì bỏ qua).
- Lệnh trên channel riêng:
  `kubectl get pod -A --context '<ctx>' 2>&1 | grep -i -F -- '<kw>' | head -n 200; echo __LAZY_SEARCH_<id>__`
- `<kw>` bọc trong nháy đơn, escape `'` → `'\''`. Timeout 15s.
- Parse: bỏ dòng header `NAMESPACE`, tách cột bằng ≥2 khoảng trắng → Namespace | Pod | Ready | Status | Restarts | Age.
- Status tô màu: Running xanh, Pending vàng, Error/CrashLoopBackOff/ImagePullBackOff đỏ, Succeeded/Completed xám.
- Kết quả bị cắt ở 200 dòng → cảnh báo "Kết quả quá nhiều — thu hẹp từ khóa"; 0 dòng → empty state.
- Nút Refresh chạy lại search hiện tại; click header để sort client-side.
- Chọn 1 pod → mở tab log.

## 8. Tab log
- Trước khi mở: lấy danh sách container bằng
  `kubectl get pod '<pod>' -n '<ns>' --context '<ctx>' -o jsonpath='{range .spec.containers[*]}{.name}{"\n"}{end}'`
  — 1 container thì mở luôn, nhiều thì hiện dropdown chọn.
- Lệnh: `kubectl -n '<ns>' --context '<ctx>' logs -f -c '<c>' --tail=<N> [--timestamps] [--since=<S>] '<pod>' 2>&1`
- N mặc định 500 (chỉnh được); timestamps/since áp dụng cho lần mở/reconnect kế tiếp.
- `logs -f` không bao giờ tự kết thúc → không chờ marker; chờ tối đa 3s để phát hiện lỗi khởi động (`NotFound`, `container not found`…) rồi chuyển sang chế độ stream.
- Stream: backend gom batch, emit event `log:<tabId>` mỗi **100ms** (mảng dòng raw ANSI) tránh flood IPC.
- Hiển thị: monospace, render ANSI giữ màu; auto-scroll bật mặc định, tự tắt khi user cuộn lên + nút "Xuống cuối".
- Tiện ích: tìm trong log (highlight + nhảy), xóa màn hình (clear buffer, không dừng stream), lưu file (dialog, mặc định `%USERPROFILE%\Downloads\<ns>_<pod>_<timestamp>.log`, lưu tối đa số dòng đang giữ).
- Ring buffer **10.000 dòng cuối**.
- Đóng tab → đóng channel. Stream kết thúc (pod bị xóa/restart) → hiện "Stream kết thúc — pod có thể đã bị xóa" + nút Thử lại (re-run chuỗi + `logs -f --tail=100`). Mất kết nối → tab báo lỗi + nút Kết nối lại.
- Chỉ 1 profile active; đổi profile → confirm đóng các tab.

## 9. Kiến trúc
- Backend Rust (Tauri 2): `russh`, `russh-keys`, `aes-gcm`, `argon2`, `keyring`, `tokio`, `zeroize`, `uuid`, `serde`, `thiserror`.
- Frontend: Vite + React 18 + TypeScript, Tailwind CSS, **Zustand** (state), **ansi-to-react** (render log).
- Commands (Tauri): `profile_list`, `profile_save`, `profile_delete`, `profile_duplicate`, `store_change_key`, `test_connection`, `get_contexts`, `connect`, `disconnect`, `search_pods`, `open_log_tab`, `close_log_tab`, `set_log_options`.
- Events: `conn:<profileId>` (trạng thái), `search:<requestId>` (kết quả), `log:<tabId>` (batch dòng).
- Cấu trúc:
```
src/                  # frontend (React)
  components/  store/  pages/
src-tauri/src/
  lib.rs      # setup, đăng ký commands
  crypto.rs   # AES-GCM, Argon2id, keyring, file format
  profile.rs  # CRUD + persistence
  ssh.rs      # connection pool, PTY, marker engine, keepalive
  kubectl.rs  # command builders + output parsing
  error.rs    # lỗi phân loại → message tiếng Việt
```

## 10. Milestones
| # | Phạm vi | Xong khi |
|---|---------|----------|
| M1 | Scaffold Tauri + React, Profile CRUD + mã hóa + keyring/passphrase + unlock | Tạo/sửa/xóa profile; file mã hóa roundtrip; unit test crypto pass |
| M2 | SSH connect: auth, TOFU, chuỗi sudo/su/kubectl, trạng thái, reconnect, Test kết nối | Connect thật tới server; các lỗi hiện đúng message |
| M3 | Tìm pod | Debounce + parse bảng + tô màu + limit + empty state |
| M4 | Tab log | Stream realtime, multi-container, auto-scroll, tìm, lưu file, reconnect tab |
| M5 | Polish + build | Icon, message tiếng Việt đủ, build ra NSIS/MSI |

## 11. Kiểm thử
- **Unit (Rust)**: crypto roundtrip + sai key fail + AAD; marker engine (giả lập output); escape grep; parse output kubectl; version file format.
- **Integration**: mock SSH server bằng russh server API (auth password, mô phỏng sudo) — test chuỗi marker không cần server thật.
- **Manual checklist**: sai mật khẩu, host sai/không đạt, fingerprint đổi, kubectl thiếu, context sai, pod nhiều container, pod bị xóa khi đang log, mất mạng giữa chừng, đổi profile qua lại, đổi key.

## 12. Build & phân phối
- Windows 10/11 x64; `npm run tauri build` → NSIS `.exe` setup + MSI.
- Không code signing (dùng cá nhân, chấp nhận cảnh báo SmartScreen).
- Tên app: "SSH UI Lazy", identifier: `com.sshuilazy.app`.

## 13. Cần confirm trước khi triển khai
1. ~~Context~~ — **CHỐT**: dropdown từ `get-contexts` sau khi Test kết nối + cho nhập tay.
2. ~~Theme~~ — **CHỐT**: toggle dark/light, lưu localStorage.
3. ~~Limit kết quả tìm pod 200 dòng~~ — **CHỐT** (dùng mặc định).
4. ~~Argon2id m=64MiB / t=3 / p=4~~ — **CHỐT** (dùng mặc định).
5. ~~Log tail mặc định 500, lưu file vào Downloads~~ — **CHỐT** (dùng mặc định).

## 14. Ghi chú triển khai (đã xong)
- Kết quả tìm pod trả về **trực tiếp qua response của command** `search_pods` (generation counter phía frontend vẫn loại kết quả stale) thay vì event `search:<requestId>`.
- Timestamp ISO trong profile viết thủ công (không thêm crate thời gian).
- russh dùng backend `ring` (aws-lc-rs cần NASM, không có trên máy build).
- Commands đã đăng ký: `store_status`, `store_unlock`, `store_change_key`, `profile_list`, `profile_save`, `profile_delete`, `profile_duplicate`, `test_connection`, `connect`, `disconnect`, `get_contexts`, `search_pods`, `open_log_tab`, `start_log_stream`, `set_log_options`, `close_log_tab`, `save_log_file`.
- Events: `conn:<profileId>`, `log:<tabId>`, `logend:<tabId>`.
