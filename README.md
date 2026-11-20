# SSH UI Lazy

Ứng dụng desktop **Windows** (Tauri 2) để xem log Kubernetes qua SSH: đăng nhập SSH bằng mật khẩu, vào môi trường `root` → `gitlab-runner`, chọn kubectl context, tìm kiếm pod và xem log realtime nhiều tab.

## Tính năng

- **Quản lý profile**: thêm / sửa / xóa / nhân bản / test kết nối. Profile được mã hóa cục bộ bằng AES-256-GCM (key lưu OS keyring hoặc derive từ passphrase bằng Argon2id).
- **Kết nối SSH**: auth bằng mật khẩu (russh), xác thực fingerprint kiểu TOFU, tự chạy chuỗi `set +H` → `sudo -i` → `su - gitlab-runner` → kiểm tra kubectl bằng marker engine.
- **Tìm pod**: debounce khi nhập ≥3 ký tự, giới hạn 200 dòng kết quả, tô màu theo status, sort theo cột.
- **Tab log realtime**: stream `kubectl logs -f` qua channel SSH riêng, nhiều tab song song, multi-container, auto-scroll, tìm kiếm trong log, lưu file, ring buffer 10.000 dòng, render ANSI màu.
- **Dark / light theme**, lưu localStorage.

## Công nghệ

| Phần | Công nghệ |
|------|-----------|
| Frontend | Vite + React 18 + TypeScript, Tailwind CSS, Zustand, ansi-to-react |
| Backend | Rust (Tauri 2): russh, aes-gcm, argon2, keyring, tokio, zeroize |
| Package | NSIS `.exe` setup + MSI |

## Cấu trúc

```
src/                  # frontend (React)
  components/  store/  test/
src-tauri/src/
  lib.rs      # setup, đăng ký commands
  crypto.rs   # AES-GCM, Argon2id, keyring, file format
  profile.rs  # CRUD + persistence
  ssh.rs      # connection pool, PTY, marker engine, keepalive
  kubectl.rs  # command builders + output parsing
  error.rs    # lỗi phân loại → message tiếng Việt
```

## Cài đặt & chạy

Yêu cầu: Node.js, npm, Rust toolchain (Windows 10/11 x64).

```bash
# cài dependencies
npm install

# chạy chế độ dev
npm run tauri dev

# build installer (NSIS + MSI)
npm run tauri build
```

## Script

| Lệnh | Mô tả |
|------|-------|
| `npm run dev` | Vite dev server (frontend) |
| `npm run build` | Type-check + build frontend |
| `npm run tauri dev` | Chạy app desktop chế độ dev |
| `npm run tauri build` | Build installer NSIS/MSI |
| `npm run test` | Chạy unit test (vitest) |
| `npm run test:watch` | Chạy test ở chế độ watch |

## Vị trí lưu dữ liệu

- `%APPDATA%\ssh-ui-lazy\profiles.dat` — file profile mã hóa
- `%APPDATA%\ssh-ui-lazy\known_hosts.json` — fingerprint host (TOFU)
- Log lưu mặc định vào `%USERPROFILE%\Downloads`

## Ngoài phạm vi (non-goals)

- SSH key auth, 2FA/OTP, bastion/jump host, agent forwarding
- Chạy nhiều profile active cùng lúc
- Code signing installer, auto-update
- Quản lý/sửa kubeconfig, chạy lệnh kubectl tùy ý

Chi tiết thiết kế xem [PLAN.md](PLAN.md).

Link tải:
 https://drive.google.com/file/d/1Rt-w1G6-heFcNozGZm9ebXX23-P16gHF/view?usp=drive_link
https://drive.google.com/file/d/10rKoOPuFjL9T7WFLFKcIFowbkPVf75TL/view?usp=drive_link