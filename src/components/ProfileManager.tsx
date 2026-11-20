import { useEffect, useState } from "react";
import { api, AppError, errMessage } from "../api";
import { useStore } from "../store/useStore";
import type { ProfileInput } from "../types";
import Modal from "./Modal";

const inputCls =
  "w-full rounded-md border border-slate-300 bg-white px-3 py-2 text-sm outline-none focus:border-blue-500 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100";
const labelCls = "mb-1 block text-sm font-medium text-slate-600 dark:text-slate-300";
const btnPrimary =
  "rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-700 disabled:opacity-50";
const btnSecondary =
  "rounded-md border border-slate-300 px-4 py-2 text-sm font-medium hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-800";

export function ProfileForm({
  initial,
  onSaved,
  onCancel,
}: {
  initial?: ProfileInput & { id?: string };
  onSaved: () => void;
  onCancel?: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [host, setHost] = useState(initial?.host ?? "");
  const [port, setPort] = useState(initial?.port != null ? String(initial.port) : "");
  const [username, setUsername] = useState(initial?.username ?? "");
  const [password, setPassword] = useState(initial?.password ?? "");
  const [context, setContext] = useState(initial?.context ?? "");
  const [contexts, setContexts] = useState<string[]>([]);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ kind: "ok" | "err"; text: string } | null>(null);

  const ctxOptions = contexts.length > 0 ? contexts : initial?.context ? [initial.context] : [];

  async function runTest(trustNewHost: boolean) {
    setTesting(true);
    setMsg(null);
    try {
      const result = await api.testConnection({
        host,
        port: port ? Number(port) : 22,
        username,
        password,
        trustNewHost,
      });
      setContexts(result.contexts);
      if (!context && result.contexts.length > 0) setContext(result.contexts[0]);
      setMsg({
        kind: "ok",
        text: `Kết nối OK — fingerprint ${result.fingerprint}. Tìm được ${result.contexts.length} context.`,
      });
    } catch (e) {
      if (e instanceof AppError && e.code === "HOST_UNKNOWN" && e.fingerprint) {
        const { confirm } = await import("@tauri-apps/plugin-dialog");
        const ok = await confirm(
          `Máy chủ chưa được tin cậy.\n\nFingerprint: ${e.fingerprint}\n\nTin tưởng máy chủ này?`,
          { title: "Xác nhận host mới", kind: "warning" },
        );
        if (ok) {
          setTesting(false);
          await runTest(true);
          return;
        }
      }
      setMsg({ kind: "err", text: errMessage(e) });
    } finally {
      setTesting(false);
    }
  }

  async function save() {
    setSaving(true);
    setMsg(null);
    try {
      await api.profileSave({
        id: initial?.id ?? null,
        name,
        host,
        port: port ? Number(port) : null,
        username: username || null,
        password,
        context,
      });
      onSaved();
    } catch (e) {
      setMsg({ kind: "err", text: errMessage(e) });
    } finally {
      setSaving(false);
    }
  }

  const canSave = name.trim() && host.trim() && password && context.trim();

  return (
    <form
      className="space-y-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (canSave && !saving) void save();
      }}
    >
      <div>
        <label className={labelCls}>Tên profile *</label>
        <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} placeholder="staging-126" required />
      </div>
      <div className="grid grid-cols-4 gap-3">
        <div className="col-span-3">
          <label className={labelCls}>SSH host * (user@host:port)</label>
          <input className={inputCls} value={host} onChange={(e) => setHost(e.target.value)} placeholder="abc@10.110.30.126:22" required />
        </div>
        <div>
          <label className={labelCls}>Port</label>
          <input className={inputCls} value={port} onChange={(e) => setPort(e.target.value)} placeholder="22" inputMode="numeric" />
        </div>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className={labelCls}>Username</label>
          <input className={inputCls} value={username} onChange={(e) => setUsername(e.target.value)} placeholder="abc (bỏ trống nếu có trong host)" />
        </div>
        <div>
          <label className={labelCls}>Mật khẩu *</label>
          <input className={inputCls} type="password" value={password} onChange={(e) => setPassword(e.target.value)} required />
        </div>
      </div>
      <div>
        <label className={labelCls}>Context *</label>
        <div className="flex gap-2">
          {ctxOptions.length > 0 ? (
            <select className={inputCls} value={context} onChange={(e) => setContext(e.target.value)}>
              {!ctxOptions.includes(context) && context && <option value={context}>{context}</option>}
              {ctxOptions.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          ) : (
            <input className={inputCls} value={context} onChange={(e) => setContext(e.target.value)} placeholder="staging-cluster" />
          )}
          <button type="button" className={btnSecondary + " whitespace-nowrap"} onClick={() => void runTest(false)} disabled={testing || !host.trim() || !password}>
            {testing ? "Đang test…" : "Test kết nối"}
          </button>
        </div>
        <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">
          Nhập tay hoặc bấm Test kết nối để lấy danh sách context.
        </p>
      </div>

      {msg && (
        <p className={`text-sm ${msg.kind === "ok" ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"}`}>
          {msg.text}
        </p>
      )}

      <div className="flex justify-end gap-2 pt-2">
        {onCancel && (
          <button type="button" className={btnSecondary} onClick={onCancel}>
            Hủy
          </button>
        )}
        <button type="submit" className={btnPrimary} disabled={!canSave || saving}>
          {saving ? "Đang lưu…" : "Lưu profile"}
        </button>
      </div>
    </form>
  );
}

// Danh sách profile + nút thao tác (mục 2.2 PLAN)
export function ProfileList({ onChanged }: { onChanged: () => void }) {
  const { profiles, setManagerOpen } = useStore();
  const [editing, setEditing] = useState<ProfileInput & { id?: string } | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function del(p: { id: string; name: string }) {
    const { confirm } = await import("@tauri-apps/plugin-dialog");
    const ok = await confirm(`Xóa profile '${p.name}'?`, { title: "Xác nhận xóa", kind: "warning" });
    if (!ok) return;
    try {
      await api.profileDelete(p.id);
      onChanged();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  async function dup(p: { id: string }) {
    try {
      await api.profileDuplicate(p.id);
      onChanged();
    } catch (e) {
      setError(errMessage(e));
    }
  }

  if (editing || creating) {
    return (
      <div>
        <h3 className="mb-3 text-sm font-semibold">{creating ? "Thêm profile" : "Sửa profile"}</h3>
        <ProfileForm
          initial={editing ?? undefined}
          onSaved={() => {
            setEditing(null);
            setCreating(false);
            onChanged();
          }}
          onCancel={() => {
            setEditing(null);
            setCreating(false);
          }}
        />
      </div>
    );
  }

  return (
    <div>
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold">Danh sách profile</h3>
        <button className={btnPrimary} onClick={() => setCreating(true)}>
          + Thêm
        </button>
      </div>
      {error && <p className="mb-2 text-sm text-red-600 dark:text-red-400">{error}</p>}
      <ul className="space-y-2">
        {profiles.map((p) => (
          <li
            key={p.id}
            className="flex items-center justify-between rounded-md border border-slate-200 px-3 py-2 dark:border-slate-700"
          >
            <div>
              <div className="text-sm font-medium">{p.name}</div>
              <div className="text-xs text-slate-500 dark:text-slate-400">
                {p.username}@{p.host}:{p.port} → {p.context}
              </div>
            </div>
            <div className="flex gap-1 text-xs">
              <button className="rounded px-2 py-1 hover:bg-slate-100 dark:hover:bg-slate-800" onClick={() => setEditing(p)}>
                Sửa
              </button>
              <button className="rounded px-2 py-1 hover:bg-slate-100 dark:hover:bg-slate-800" onClick={() => void dup(p)}>
                Nhân bản
              </button>
              <button
                className="rounded px-2 py-1 text-red-600 hover:bg-red-50 dark:text-red-400 dark:hover:bg-red-950"
                onClick={() => void del(p)}
              >
                Xóa
              </button>
            </div>
          </li>
        ))}
        {profiles.length === 0 && (
          <li className="py-4 text-center text-sm text-slate-500 dark:text-slate-400">Chưa có profile nào.</li>
        )}
      </ul>
      <ChangeKeySection />
      <div className="mt-3 flex justify-end">
        <button className={btnSecondary} onClick={() => setManagerOpen(false)}>
          Đóng
        </button>
      </div>
    </div>
  );
}

function ChangeKeySection() {
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const [showPass, setShowPass] = useState(false);
  const [pass, setPass] = useState("");

  async function changeKey(newPassphrase: string | null) {
    setBusy(true);
    setMsg(null);
    try {
      await api.storeChangeKey(newPassphrase);
      setMsg(
        newPassphrase
          ? "Đã chuyển sang mã hóa bằng passphrase. Cảnh báo: quên key = mất dữ liệu!"
          : "Đã chuyển sang key hệ thống (Windows Credential Manager).",
      );
      setPass("");
      setShowPass(false);
    } catch (e) {
      setMsg(errMessage(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="mt-4 rounded-md border border-slate-200 p-3 dark:border-slate-700">
      <h4 className="text-sm font-semibold">Khóa mã hóa dữ liệu</h4>
      <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">
        Mặc định: key 32B trong Windows Credential Manager. Có thể đổi sang passphrase (Argon2id).
      </p>
      {showPass ? (
        <div className="mt-2 flex gap-2">
          <input
            className={inputCls}
            type="password"
            placeholder="Passphrase mới"
            value={pass}
            onChange={(e) => setPass(e.target.value)}
          />
          <button className={btnPrimary + " whitespace-nowrap"} disabled={busy || !pass} onClick={() => void changeKey(pass)}>
            Xác nhận
          </button>
          <button className={btnSecondary} onClick={() => setShowPass(false)}>
            Hủy
          </button>
        </div>
      ) : (
        <div className="mt-2 flex gap-2">
          <button className={btnSecondary} disabled={busy} onClick={() => setShowPass(true)}>
            Dùng passphrase…
          </button>
          <button className={btnSecondary} disabled={busy} onClick={() => void changeKey(null)}>
            Dùng key hệ thống
          </button>
        </div>
      )}
      {msg && <p className="mt-2 text-xs text-blue-600 dark:text-blue-400">{msg}</p>}
    </div>
  );
}

export default function ProfileManagerModal() {
  const { setManagerOpen, loadProfiles } = useStore();
  useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);
  return (
    <Modal title="Quản lý profile" onClose={() => setManagerOpen(false)} wide>
      <ProfileList onChanged={() => void loadProfiles()} />
    </Modal>
  );
}
