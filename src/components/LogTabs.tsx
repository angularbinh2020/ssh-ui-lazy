import { memo, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import AnsiDefault from "ansi-to-react";
import { useStore } from "../store/useStore";
import type { LogTab } from "../types";
import { api, errMessage } from "../api";

// ansi-to-react là CJS thuần (__esModule + exports.default) nên Vite dev trả
// default import = cả module object; phải unwrap thủ công (dev/build/test đều chạy đúng)
type AnsiProps = { children?: string };
type AnsiModule = ComponentType<AnsiProps> & { default?: ComponentType<AnsiProps> };
const _AnsiModule = AnsiDefault as AnsiModule;
const Ansi: ComponentType<AnsiProps> = _AnsiModule.default ?? _AnsiModule;

const RENDER_LIMIT = 2000;

const DEFAULT_HEIGHT_PCT = 45;
const MIN_HEIGHT_PCT = 15;
const MAX_HEIGHT_PCT = 85;

export default function LogTabs() {
  const { tabs, activeTabId, setActiveTab, closeTab } = useStore();
  const [heightPct, setHeightPct] = useState(DEFAULT_HEIGHT_PCT);
  const [maximized, setMaximized] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  if (tabs.length === 0) return null;
  const active = tabs.find((t) => t.id === activeTabId) ?? tabs[0];

  // kéo viền trên khung log để tăng/giảm chiều cao (PodSearch bên trên co giãn theo)
  function onResizeStart(e: React.PointerEvent) {
    e.preventDefault();
    const container = containerRef.current;
    if (!container) return;
    const move = (ev: PointerEvent) => {
      const parent = container.parentElement;
      if (!parent) return;
      const rect = parent.getBoundingClientRect();
      if (rect.height <= 0) return;
      const pct = ((rect.bottom - ev.clientY) / rect.height) * 100;
      setHeightPct(Math.min(MAX_HEIGHT_PCT, Math.max(MIN_HEIGHT_PCT, pct)));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.style.userSelect = "";
    };
    document.body.style.userSelect = "none";
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  return (
    <div
      ref={containerRef}
      className={
        maximized
          ? "absolute inset-0 z-20 flex min-h-0 flex-col border-t border-slate-200 bg-white dark:border-slate-800 dark:bg-slate-950"
          : "flex min-h-0 flex-col border-t border-slate-200 dark:border-slate-800"
      }
      style={maximized ? undefined : { height: `${heightPct}%` }}
    >
      {!maximized && (
        <div
          className="h-1.5 w-full shrink-0 cursor-row-resize hover:bg-blue-200 dark:hover:bg-blue-900/60"
          onPointerDown={onResizeStart}
          onDoubleClick={() => setHeightPct(DEFAULT_HEIGHT_PCT)}
          title="Kéo để thay đổi chiều cao khung log (double-click để về mặc định)"
        />
      )}
      <div className="flex items-center gap-1 overflow-x-auto border-b border-slate-200 bg-slate-50 px-2 dark:border-slate-800 dark:bg-slate-900">
        {tabs.map((t) => (
          <div
            key={t.id}
            className={`flex cursor-pointer items-center gap-2 rounded-t-md px-3 py-1.5 text-xs ${
              t.id === active.id
                ? "bg-white font-semibold text-blue-700 dark:bg-slate-800 dark:text-blue-400"
                : "text-slate-500 hover:bg-slate-100 dark:hover:bg-slate-800"
            }`}
            onClick={() => setActiveTab(t.id)}
          >
            <span>
              {t.ns}/{t.pod}
            </span>
            {t.status === "streaming" && <span className="h-1.5 w-1.5 rounded-full bg-green-500" />}
            {t.status === "error" && <span className="h-1.5 w-1.5 rounded-full bg-red-500" />}
            <button
              className="text-slate-400 hover:text-red-500"
              onClick={(e) => {
                e.stopPropagation();
                void closeTab(t.id);
              }}
              aria-label={`Đóng tab ${t.pod}`}
            >
              ✕
            </button>
          </div>
        ))}
        <button
          className="ml-auto shrink-0 rounded px-2 py-0.5 text-xs text-slate-500 hover:bg-slate-100 hover:text-slate-800 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100"
          onClick={() => setMaximized((v) => !v)}
          title={maximized ? "Thu nhỏ khung log" : "Phóng to khung log"}
          aria-label={maximized ? "Thu nhỏ khung log" : "Phóng to khung log"}
        >
          {maximized ? "🗗 thu nhỏ" : "⛶ phóng to"}
        </button>
      </div>
      <LogTabPanel key={active.id} tab={active} />
    </div>
  );
}

// memo: tab khác thay đổi lines không làm panel tab này re-render
const LogTabPanel = memo(function LogTabPanel({ tab }: { tab: LogTab }) {
  const {
    chooseContainer,
    retryTab,
    clearTabLines,
    setTabAutoScroll,
    setTabOptions,
    conn,
    reconnect,
  } = useStore();

  const [showOpts, setShowOpts] = useState(false);
  const [searchTerm, setSearchTerm] = useState("");
  const [matchCursor, setMatchCursor] = useState(0);
  const [tail, setTail] = useState(String(tab.options.tail));
  const [timestamps, setTimestamps] = useState(tab.options.timestamps);
  const [since, setSince] = useState(tab.options.since ?? "");
  const [saving, setSaving] = useState(false);
  const viewerRef = useRef<HTMLDivElement>(null);

  // tìm kiếm chỉ trên cửa sổ render (RENDER_LIMIT dòng cuối) — data-line chỉ
  // tồn tại cho các dòng hiển thị nên jump ngoài cửa sổ sẽ vô hiệu
  const displayLines = tab.lines.slice(-RENDER_LIMIT);
  const hiddenCount = tab.lines.length - displayLines.length;
  const highlight = searchTerm.trim().length > 0;

  const matchIdx = useMemo(() => {
    if (!searchTerm) return [] as number[];
    const term = searchTerm.toLowerCase();
    const out: number[] = [];
    for (let i = hiddenCount; i < tab.lines.length; i++) {
      if (tab.lines[i].toLowerCase().includes(term)) out.push(i);
    }
    return out;
  }, [searchTerm, tab.lines, hiddenCount]);

  useEffect(() => {
    setMatchCursor(0);
  }, [searchTerm]);

  // auto-scroll: giữ ở đáy khi có dòng mới (mục 8 PLAN)
  useEffect(() => {
    if (tab.autoScroll && viewerRef.current) {
      viewerRef.current.scrollTop = viewerRef.current.scrollHeight;
    }
  }, [tab.lines, tab.autoScroll]);

  function jumpToMatch(dir: 1 | -1) {
    if (matchIdx.length === 0 || !viewerRef.current) return;
    const next = (matchCursor + dir + matchIdx.length) % matchIdx.length;
    setMatchCursor(next);
    const idx = matchIdx[next];
    const el = viewerRef.current.querySelector(`[data-line="${idx}"]`);
    el?.scrollIntoView({ block: "center" });
  }

  async function saveToFile() {
    setSaving(true);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const stamp = (() => {
        const d = new Date();
        const pad = (n: number) => String(n).padStart(2, "0");
        return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(
          d.getHours(),
        )}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
      })();
      const path = await save({
        title: "Lưu log",
        defaultPath: `${tab.ns}_${tab.pod}_${stamp}.log`,
      });
      if (path) {
        await api.saveLogFile(path, tab.lines.join("\n") + "\n");
      }
    } catch (e) {
      alert(`Lỗi lưu file: ${errMessage(e)}`);
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col bg-white dark:bg-slate-950">
      {/* toolbar — mục 8 PLAN */}
      <div className="flex flex-wrap items-center gap-2 border-b border-slate-100 px-3 py-1.5 text-xs dark:border-slate-900">
        <label className="flex items-center gap-1 cursor-pointer">
          <input
            type="checkbox"
            checked={tab.autoScroll}
            onChange={(e) => setTabAutoScroll(tab.id, e.target.checked)}
          />
          auto-scroll
        </label>
        {!tab.autoScroll && (
          <button
            className="rounded bg-blue-600 px-2 py-0.5 text-white hover:bg-blue-700"
            onClick={() => setTabAutoScroll(tab.id, true)}
          >
            ↓ Xuống cuối
          </button>
        )}
        <div className="flex items-center gap-1">
          <input
            className="w-40 rounded border border-slate-300 px-2 py-0.5 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
            placeholder="🔍 tìm trong log"
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") jumpToMatch(e.shiftKey ? -1 : 1);
            }}
          />
          {highlight && (
            <span className="text-slate-500 dark:text-slate-400">
              {matchIdx.length > 0 ? `${matchCursor + 1}/${matchIdx.length}` : "0"}
            </span>
          )}
        </div>
        <button
          className="rounded border border-slate-300 px-2 py-0.5 hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-800"
          onClick={() => clearTabLines(tab.id)}
          title="Xóa màn hình (không dừng stream)"
        >
          🧹 xóa
        </button>
        <button
          className="rounded border border-slate-300 px-2 py-0.5 hover:bg-slate-100 disabled:opacity-40 dark:border-slate-600 dark:hover:bg-slate-800"
          onClick={() => void saveToFile()}
          disabled={saving || tab.lines.length === 0}
          title="Lưu ra file"
        >
          💾 lưu
        </button>
        <button
          className="rounded border border-slate-300 px-2 py-0.5 hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-800"
          onClick={() => setShowOpts((v) => !v)}
          title="Tùy chọn log"
        >
          ⚙ opts
        </button>
        <span className="ml-auto text-slate-400">
          {tab.lines.length} dòng{hiddenCount > 0 ? ` (hiển thị ${RENDER_LIMIT} cuối)` : ""}
        </span>
      </div>

      {showOpts && (
        <div className="flex flex-wrap items-center gap-3 border-b border-slate-100 bg-slate-50 px-3 py-2 text-xs dark:border-slate-900 dark:bg-slate-900">
          <label className="flex items-center gap-1">
            tail:
            <input
              className="w-20 rounded border border-slate-300 px-1.5 py-0.5 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
              value={tail}
              onChange={(e) => setTail(e.target.value)}
              inputMode="numeric"
            />
          </label>
          <label className="flex items-center gap-1">
            <input type="checkbox" checked={timestamps} onChange={(e) => setTimestamps(e.target.checked)} />
            timestamps
          </label>
          <label className="flex items-center gap-1">
            since:
            <input
              className="w-24 rounded border border-slate-300 px-1.5 py-0.5 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
              placeholder="5m / 1h…"
              value={since}
              onChange={(e) => setSince(e.target.value)}
            />
          </label>
          <button
            className="rounded bg-slate-700 px-2 py-0.5 text-white hover:bg-slate-600"
            onClick={() => {
              setTabOptions(tab.id, {
                tail: Math.max(1, Number(tail) || 500),
                timestamps,
                since: since.trim() || null,
              });
            }}
          >
            Áp dụng lần mở/reconnect kế tiếp
          </button>
        </div>
      )}

      {/* trạng thái — mục 8 PLAN */}
      {tab.status === "select-container" && (
        <div className="flex items-center gap-2 px-3 py-2 text-sm">
          Chọn container:
          {tab.containers.length === 0 ? (
            <span className="text-slate-500">đang tải…</span>
          ) : (
            <select
              className="rounded border border-slate-300 px-2 py-1 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
              defaultValue=""
              onChange={(e) => {
                if (e.target.value) void chooseContainer(tab.id, e.target.value);
              }}
            >
              <option value="" disabled>
                — chọn —
              </option>
              {tab.containers.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
          )}
        </div>
      )}
      {tab.status === "starting" && (
        <div className="px-3 py-2 text-sm text-yellow-600 dark:text-yellow-400">Đang mở stream…</div>
      )}
      {tab.status === "ended" && (
        <div className="flex items-center gap-3 px-3 py-2 text-sm text-yellow-600 dark:text-yellow-400">
          Stream kết thúc — pod có thể đã bị xóa.
          <button
            className="rounded bg-blue-600 px-2 py-0.5 text-white hover:bg-blue-700"
            onClick={() => void retryTab(tab.id)}
          >
            Thử lại
          </button>
        </div>
      )}
      {tab.status === "error" && (
        <div className="flex items-center gap-3 px-3 py-2 text-sm text-red-600 dark:text-red-400">
          {tab.error}
          <button
            className="rounded bg-blue-600 px-2 py-0.5 text-white hover:bg-blue-700"
            onClick={() => void retryTab(tab.id)}
          >
            Thử lại
          </button>
          {conn.state !== "ready" && conn.state !== "connecting" && (
            <button
              className="rounded border border-slate-300 px-2 py-0.5 dark:border-slate-600"
              onClick={() => void reconnect()}
            >
              Kết nối lại
            </button>
          )}
        </div>
      )}

      {/* log viewer */}
      <div
        ref={viewerRef}
        className="log-view min-h-0 flex-1 overflow-auto px-3 py-2 text-slate-800 dark:text-slate-200"
        onWheel={(e) => {
          // cuộn lên → tự tắt auto-scroll (mục 8 PLAN)
          if (e.deltaY < 0 && tab.autoScroll) {
            setTabAutoScroll(tab.id, false);
          }
        }}
      >
        {tab.lines.length === 0 ? (
          <div className="text-slate-400">
            {tab.status === "streaming" ? "Chưa có log mới…" : "—"}
          </div>
        ) : (
          <>
            {hiddenCount > 0 && (
              <div className="text-slate-400">… {hiddenCount} dòng cũ hơn (giữ tối đa 10.000 dòng) …</div>
            )}
            {displayLines.map((line, i) => {
              const idx = hiddenCount + i;
              const matched = highlight && line.toLowerCase().includes(searchTerm.toLowerCase());
              return (
                <div
                  key={idx}
                  data-line={idx}
                  className={matched ? "rounded bg-yellow-200 dark:bg-yellow-700/40" : undefined}
                >
                  <Ansi>{line || " "}</Ansi>
                </div>
              );
            })}
          </>
        )}
      </div>
    </div>
  );
});
