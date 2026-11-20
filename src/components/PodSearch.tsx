import { useEffect, useMemo, useRef, useState } from "react";
import { useStore } from "../store/useStore";
import { statusColor, type PodRow } from "../types";

type SortKey = "namespace" | "pod" | "ready" | "status" | "restarts" | "age";

const COLUMNS: { key: SortKey; label: string }[] = [
  { key: "namespace", label: "NS" },
  { key: "pod", label: "Pod" },
  { key: "ready", label: "Ready" },
  { key: "status", label: "Status" },
  { key: "restarts", label: "Restarts" },
  { key: "age", label: "Age" },
];

export default function PodSearch() {
  const { keyword, setKeyword, search, runSearch, conn, openTab } = useStore();
  const [sort, setSort] = useState<{ key: SortKey; asc: boolean }>({ key: "namespace", asc: true });
  const debounceRef = useRef<number | null>(null);
  const lastRunRef = useRef<string>("");

  // debounce 400ms, ≥3 ký tự — mục 7 PLAN
  useEffect(() => {
    if (debounceRef.current) window.clearTimeout(debounceRef.current);
    if (keyword.trim().length < 3) {
      lastRunRef.current = "";
      return;
    }
    debounceRef.current = window.setTimeout(() => {
      lastRunRef.current = keyword.trim();
      void runSearch();
    }, 400);
    return () => {
      if (debounceRef.current) window.clearTimeout(debounceRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keyword]);

  // refresh khi kết nối chuyển sang ready
  useEffect(() => {
    if (conn.state === "ready" && keyword.trim().length >= 3) {
      void runSearch();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn.state]);

  const rows = useMemo(() => {
    const { rows } = search;
    const sorted = [...rows].sort((a, b) => {
      const va = a[sort.key] ?? "";
      const vb = b[sort.key] ?? "";
      const cmp = va.localeCompare(vb, undefined, { numeric: true });
      return sort.asc ? cmp : -cmp;
    });
    return sorted;
  }, [search.rows, sort]);

  function clickRow(p: PodRow) {
    void openTab(p.namespace, p.pod);
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-4">
      <div className="flex items-center gap-2">
        <input
          className="w-full max-w-md rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm outline-none focus:border-blue-500 dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100"
          placeholder="Tìm pod (≥3 ký tự, theo tên/namespace…)"
          value={keyword}
          onChange={(e) => setKeyword(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void runSearch();
          }}
        />
        <button
          className="whitespace-nowrap rounded-md border border-slate-300 px-3 py-1.5 text-sm hover:bg-slate-100 disabled:opacity-40 dark:border-slate-600 dark:hover:bg-slate-800"
          onClick={() => void runSearch()}
          disabled={keyword.trim().length < 3 || conn.state !== "ready"}
          title="Refresh"
        >
          Refresh
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-auto rounded-md border border-slate-200 dark:border-slate-700">
        {conn.state !== "ready" ? (
          <Empty text={conn.state === "connecting" ? "Đang kết nối…" : "Chưa kết nối — chọn profile và bấm Kết nối lại."} />
        ) : search.loading ? (
          <Empty text="Đang tìm…" />
        ) : search.error ? (
          <Empty text={`Lỗi: ${search.error}`} error />
        ) : keyword.trim().length < 3 ? (
          <Empty text="Nhập từ khóa (≥3 ký tự) để tìm pod." />
        ) : rows.length === 0 ? (
          <Empty text="Không tìm thấy pod nào." />
        ) : (
          <table className="w-full text-left text-sm">
            <thead className="sticky top-0 bg-slate-100 dark:bg-slate-800">
              <tr>
                {COLUMNS.map((c) => (
                  <th
                    key={c.key}
                    className="cursor-pointer select-none px-3 py-2 font-semibold hover:text-blue-600 dark:hover:text-blue-400"
                    onClick={() =>
                      setSort((s) => ({ key: c.key, asc: s.key === c.key ? !s.asc : true }))
                    }
                  >
                    {c.label}
                    {sort.key === c.key && (sort.asc ? " ▲" : " ▼")}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr
                  key={`${r.namespace}/${r.pod}`}
                  className="cursor-pointer border-t border-slate-100 hover:bg-blue-50 dark:border-slate-800 dark:hover:bg-slate-800"
                  onClick={() => clickRow(r)}
                >
                  <td className="px-3 py-1.5 text-slate-500 dark:text-slate-400">{r.namespace}</td>
                  <td className="px-3 py-1.5 font-medium">{r.pod}</td>
                  <td className="px-3 py-1.5">{r.ready}</td>
                  <td className={`px-3 py-1.5 font-medium ${statusColor(r.status)}`}>{r.status}</td>
                  <td className="px-3 py-1.5">{r.restarts}</td>
                  <td className="px-3 py-1.5">{r.age}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {search.truncated && (
        <p className="text-xs text-yellow-600 dark:text-yellow-400">
          Kết quả quá nhiều (giới hạn 200 dòng) — thu hẹp từ khóa.
        </p>
      )}
    </div>
  );
}

function Empty({ text, error }: { text: string; error?: boolean }) {
  return (
    <div className={`flex h-full items-center justify-center p-8 text-sm ${error ? "text-red-600 dark:text-red-400" : "text-slate-500 dark:text-slate-400"}`}>
      {text}
    </div>
  );
}
