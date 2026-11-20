import { useState } from "react";
import { useStore } from "../store/useStore";
import { ProfileForm } from "./ProfileManager";

const inputCls =
  "w-full rounded-md border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-blue-500 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100";
const btnPrimary =
  "rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50";

export default function UnlockGate() {
  const { storeStatus, bootError, unlock } = useStore();
  const [pass, setPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [localErr, setLocalErr] = useState<string | null>(null);

  // Chưa có file dữ liệu → màn hình tạo profile đầu tiên (Screen 1 — mục 2.2 PLAN)
  if (storeStatus && !storeStatus.initialized) {
    return (
      <div className="flex min-h-screen items-center justify-center p-4">
        <div className="w-full max-w-xl rounded-xl border border-slate-200 bg-white p-6 shadow-xl dark:border-slate-700 dark:bg-slate-900">
          <h1 className="mb-1 text-xl font-bold">Chào mừng đến SSH UI Lazy</h1>
          <p className="mb-5 text-sm text-slate-500 dark:text-slate-400">
            Tạo profile đầu tiên để bắt đầu. Dữ liệu được mã hóa AES-256-GCM, key lưu trong
            Windows Credential Manager.
          </p>
          {bootError && (
            <p className="mb-3 text-sm text-red-600 dark:text-red-400">{bootError}</p>
          )}
          <ProfileForm onSaved={() => void useStore.getState().init()} />
        </div>
      </div>
    );
  }

  if (!storeStatus) {
    return (
      <div className="flex min-h-screen items-center justify-center text-sm text-slate-500">
        {bootError ? bootError : "Đang tải…"}
      </div>
    );
  }

  // locked → modal mở khóa (mục 3.4 PLAN)
  return (
    <div className="flex min-h-screen items-center justify-center p-4">
      <div className="w-full max-w-md rounded-xl border border-slate-200 bg-white p-6 shadow-xl dark:border-slate-700 dark:bg-slate-900">
        <h1 className="mb-1 text-xl font-bold">Mở khóa dữ liệu</h1>
        <p className="mb-4 text-sm text-slate-500 dark:text-slate-400">
          {storeStatus.needPassphrase
            ? "Dữ liệu được mã hóa bằng passphrase. Nhập passphrase để mở khóa."
            : "Không đọc được key từ Windows Credential Manager. Kiểm tra lại hoặc khôi phục key."}
        </p>
        {bootError && <p className="mb-3 text-sm text-red-600 dark:text-red-400">{bootError}</p>}
        {localErr && <p className="mb-3 text-sm text-red-600 dark:text-red-400">{localErr}</p>}
        <form
          className="space-y-3"
          onSubmit={async (e) => {
            e.preventDefault();
            setBusy(true);
            setLocalErr(null);
            const ok = await unlock(storeStatus.needPassphrase ? pass : null);
            setBusy(false);
            if (!ok) setLocalErr("Passphrase sai hoặc dữ liệu hỏng. Thử lại.");
          }}
        >
          {storeStatus.needPassphrase && (
            <input
              className={inputCls}
              type="password"
              placeholder="Passphrase"
              value={pass}
              onChange={(e) => setPass(e.target.value)}
              autoFocus
            />
          )}
          <button type="submit" className={btnPrimary + " w-full"} disabled={busy}>
            {busy ? "Đang mở khóa…" : "Mở khóa"}
          </button>
        </form>
      </div>
    </div>
  );
}
