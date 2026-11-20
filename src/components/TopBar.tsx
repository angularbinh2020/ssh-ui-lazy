import { useStore } from "../store/useStore";
import type { ConnState } from "../types";

const STATUS_META: Record<ConnState, { label: string; cls: string }> = {
  disconnected: { label: "Mất kết nối", cls: "bg-slate-400" },
  connecting: { label: "Đang kết nối…", cls: "bg-yellow-500 animate-pulse" },
  ready: { label: "Sẵn sàng", cls: "bg-green-500" },
  error: { label: "Lỗi", cls: "bg-red-500" },
};

export default function TopBar() {
  const {
    profiles,
    activeProfileId,
    selectProfile,
    conn,
    reconnect,
    theme,
    toggleTheme,
    setManagerOpen,
  } = useStore();

  const meta = STATUS_META[conn.state];

  return (
    <div className="flex items-center gap-3 border-b border-slate-200 bg-white px-4 py-2 dark:border-slate-800 dark:bg-slate-900">
      <select
        className="max-w-56 rounded-md border border-slate-300 bg-white px-2 py-1.5 text-sm dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
        value={activeProfileId ?? ""}
        onChange={(e) => void selectProfile(e.target.value || null)}
      >
        <option value="" disabled>
          — Chọn profile —
        </option>
        {profiles.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>

      <span className="flex items-center gap-2 text-sm">
        <span className={`inline-block h-2.5 w-2.5 rounded-full ${meta.cls}`} />
        <span className="text-slate-600 dark:text-slate-300">{meta.label}</span>
        {conn.state === "error" && conn.message && (
          <span className="max-w-96 truncate text-xs text-red-600 dark:text-red-400" title={conn.message}>
            {conn.message}
          </span>
        )}
      </span>

      <div className="ml-auto flex items-center gap-2">
        <button
          className="rounded-md border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-40 dark:border-slate-600 dark:hover:bg-slate-800"
          onClick={() => void reconnect()}
          disabled={!activeProfileId || conn.state === "connecting"}
          title="Kết nối lại"
        >
          ⟳ Kết nối lại
        </button>
        <button
          className="rounded-md px-2.5 py-1.5 text-sm hover:bg-slate-100 dark:hover:bg-slate-800"
          onClick={toggleTheme}
          title="Đổi theme"
        >
          {theme === "dark" ? "☀" : "🌙"}
        </button>
        <button
          className="rounded-md px-2.5 py-1.5 text-sm hover:bg-slate-100 dark:hover:bg-slate-800"
          onClick={() => setManagerOpen(true)}
          title="Quản lý profile"
        >
          ⚙
        </button>
      </div>
    </div>
  );
}
