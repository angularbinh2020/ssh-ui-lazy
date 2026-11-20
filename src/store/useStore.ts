import { create } from "zustand";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api, errMessage } from "../api";
import type {
  ConnPayload,
  ConnState,
  LogOptions,
  LogTab,
  PodRow,
  Profile,
  StoreStatus,
} from "../types";

const RING_BUFFER_MAX = 10_000;

function uid(): string {
  return crypto.randomUUID();
}

interface SearchState {
  loading: boolean;
  rows: PodRow[];
  truncated: boolean;
  requestId: string | null;
  error: string | null;
}

interface Store {
  theme: "dark" | "light";
  storeStatus: StoreStatus | null;
  bootError: string | null;

  profiles: Profile[];
  activeProfileId: string | null;
  conn: { state: ConnState; message: string | null };

  keyword: string;
  search: SearchState;
  latestSearchId: string | null;

  tabs: LogTab[];
  activeTabId: string | null;

  managerOpen: boolean;

  init: () => Promise<void>;
  unlock: (passphrase: string | null) => Promise<boolean>;
  loadProfiles: () => Promise<void>;
  setTheme: (t: "dark" | "light") => void;
  toggleTheme: () => void;

  selectProfile: (id: string | null) => Promise<void>;
  reconnect: () => Promise<void>;

  setKeyword: (kw: string) => void;
  runSearch: () => Promise<void>;

  openTab: (ns: string, pod: string) => Promise<void>;
  chooseContainer: (tabId: string, container: string) => Promise<void>;
  closeTab: (tabId: string) => Promise<void>;
  setActiveTab: (tabId: string) => void;
  retryTab: (tabId: string) => Promise<void>;
  clearTabLines: (tabId: string) => void;
  setTabAutoScroll: (tabId: string, v: boolean) => void;
  setTabOptions: (tabId: string, opts: Partial<LogOptions>) => void;

  setManagerOpen: (open: boolean) => void;
}

const unlisteners: Record<string, UnlistenFn> = {};

async function listenOnce(
  key: string,
  event: string,
  handler: (payload: unknown) => void,
): Promise<void> {
  if (unlisteners[key]) return;
  unlisteners[key] = await listen(event, (e) => handler(e.payload));
}

export function dropListeners(prefix: string): void {
  for (const key of Object.keys(unlisteners)) {
    if (key.startsWith(prefix)) {
      unlisteners[key]!();
      delete unlisteners[key];
    }
  }
}

function trimRing(lines: string[]): string[] {
  if (lines.length <= RING_BUFFER_MAX) return lines;
  return lines.slice(lines.length - RING_BUFFER_MAX);
}

export const useStore = create<Store>((set, get) => ({
  theme:
    (localStorage.getItem("theme") as "dark" | "light" | null) ?? "dark",
  storeStatus: null,
  bootError: null,

  profiles: [],
  activeProfileId: null,
  conn: { state: "disconnected", message: null },

  keyword: "",
  search: { loading: false, rows: [], truncated: false, requestId: null, error: null },
  latestSearchId: null,

  tabs: [],
  activeTabId: null,

  managerOpen: false,

  init: async () => {
    // theme
    const theme = get().theme;
    document.documentElement.classList.toggle("dark", theme === "dark");

    // events cho từng tab được đăng ký khi mở tab (log:<id>, logend:<id>)
    try {
      const status = await api.storeStatus();
      set({ storeStatus: status });
      if (!status.initialized || !status.locked) {
        await get().loadProfiles();
      }
    } catch (e) {
      set({ bootError: errMessage(e) });
    }
  },

  unlock: async (passphrase) => {
    try {
      const profiles = await api.storeUnlock(passphrase);
      const status = await api.storeStatus();
      set({ profiles, storeStatus: status, bootError: null });
      return true;
    } catch (e) {
      set({ bootError: errMessage(e) });
      return false;
    }
  },

  loadProfiles: async () => {
    try {
      const profiles = await api.profileList();
      set({ profiles });
    } catch (e) {
      set({ bootError: errMessage(e) });
    }
  },

  setTheme: (t) => {
    localStorage.setItem("theme", t);
    document.documentElement.classList.toggle("dark", t === "dark");
    set({ theme: t });
  },

  toggleTheme: () => {
    get().setTheme(get().theme === "dark" ? "light" : "dark");
  },

  selectProfile: async (id) => {
    const prev = get().activeProfileId;
    if (prev === id) return;
    // đổi profile → confirm trước khi đóng các tab đang mở (mục 2.3 PLAN)
    if (get().tabs.length > 0) {
      const { confirm } = await import("@tauri-apps/plugin-dialog");
      const ok = await confirm(
        "Đổi profile sẽ đóng các tab log đang mở. Tiếp tục?",
        { title: "Xác nhận", kind: "warning" },
      );
      if (!ok) return;
    }
    await closeAllTabs(set, get);
    set({ activeProfileId: id, conn: { state: "disconnected", message: null } });
    if (id) {
      dropListeners("conn:");
      await listenOnce("conn:" + id, "conn:" + id, (payload) => {
        const p = payload as ConnPayload;
        set({ conn: { state: p.state, message: p.message } });
        // mất kết nối / ngắt thủ công → backend abort task stream không emit logend
        if (p.state === "disconnected" || p.state === "error") {
          set((s) => ({
            tabs: s.tabs.map((t) =>
              t.status === "streaming" || t.status === "starting"
                ? { ...t, status: "ended" }
                : t,
            ),
          }));
        }
      });
      await get().reconnect();
    }
  },

  reconnect: async () => {
    const { activeProfileId, conn } = get();
    if (!activeProfileId || conn.state === "connecting") return;
    try {
      await api.connect(activeProfileId, false);
    } catch (e) {
      const err = e as { code?: string; fingerprint?: string | null };
      if (err?.code === "HOST_UNKNOWN" && err.fingerprint) {
        // TOFU: lần đầu hiện SHA256 cho user xác nhận rồi lưu (mục 4 PLAN)
        const { confirm } = await import("@tauri-apps/plugin-dialog");
        const ok = await confirm(
          `Máy chủ chưa được tin cậy.\n\nFingerprint: ${err.fingerprint}\n\nTin tưởng máy chủ này?`,
          { title: "Xác nhận host mới", kind: "warning" },
        );
        if (ok && get().activeProfileId === activeProfileId) {
          set({ conn: { state: "connecting", message: null } });
          try {
            await api.connect(activeProfileId, true);
            return;
          } catch (e2) {
            set({ conn: { state: "error", message: errMessage(e2) } });
            return;
          }
        }
      }
      set({ conn: { state: "error", message: errMessage(e) } });
    }
  },

  setKeyword: (kw) => {
    set({ keyword: kw });
  },

  runSearch: async () => {
    const kw = get().keyword.trim();
    if (kw.length < 3 || !get().activeProfileId || get().conn.state !== "ready") {
      set({ search: { loading: false, rows: [], truncated: false, requestId: null, error: null } });
      return;
    }
    const requestId = uid();
    set((s) => ({
      latestSearchId: requestId,
      search: { ...s.search, loading: true, error: null },
    }));
    try {
      const result = await api.searchPods(requestId, kw);
      if (get().latestSearchId !== requestId) return; // stale
      set({
        search: {
          loading: false,
          rows: result.rows,
          truncated: result.truncated,
          requestId,
          error: null,
        },
      });
    } catch (e) {
      if (get().latestSearchId !== requestId) return;
      set({
        search: { loading: false, rows: [], truncated: false, requestId, error: errMessage(e) },
      });
    }
  },

  openTab: async (ns, pod) => {
    const existing = get().tabs.find((t) => t.ns === ns && t.pod === pod);
    if (existing) {
      set({ activeTabId: existing.id });
      return;
    }
    const tabId = uid();
    const tab: LogTab = {
      id: tabId,
      ns,
      pod,
      containers: [],
      container: null,
      status: "select-container",
      error: null,
      options: { tail: 500, timestamps: false, since: null },
      lines: [],
      autoScroll: true,
    };
    set((s) => ({ tabs: [...s.tabs, tab], activeTabId: tabId }));

    await listenOnce("log:" + tabId, "log:" + tabId, (payload) => {
      const p = payload as { lines: string[] };
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId ? { ...t, lines: trimRing([...t.lines, ...p.lines]) } : t,
        ),
      }));
    });
    await listenOnce("logend:" + tabId, "logend:" + tabId, () => {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId && t.status === "streaming"
            ? { ...t, status: "ended" }
            : t,
        ),
      }));
    });

    try {
      const containers = await api.openLogTab(tabId, ns, pod);
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, containers } : t)),
      }));
      if (containers.length === 1) {
        await get().chooseContainer(tabId, containers[0]);
      }
    } catch (e) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId ? { ...t, status: "error", error: errMessage(e) } : t,
        ),
      }));
    }
  },

  chooseContainer: async (tabId, container) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    if (!tab) return;
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === tabId ? { ...t, container, status: "starting", error: null } : t,
      ),
    }));
    try {
      await api.startLogStream(tabId, tab.ns, tab.pod, container, tab.options);
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId ? { ...t, status: "streaming" } : t,
        ),
      }));
    } catch (e) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId ? { ...t, status: "error", error: errMessage(e) } : t,
        ),
      }));
    }
  },

  closeTab: async (tabId) => {
    try {
      await api.closeLogTab(tabId);
    } catch {
      /* tab có thể chưa có channel — bỏ qua */
    }
    dropListeners("log:" + tabId);
    dropListeners("logend:" + tabId);
    set((s) => {
      const tabs = s.tabs.filter((t) => t.id !== tabId);
      return {
        tabs,
        activeTabId:
          s.activeTabId === tabId ? (tabs[tabs.length - 1]?.id ?? null) : s.activeTabId,
      };
    });
  },

  setActiveTab: (tabId) => set({ activeTabId: tabId }),

  retryTab: async (tabId) => {
    const tab = get().tabs.find((t) => t.id === tabId);
    if (!tab?.container) return;
    // Thử lại: re-run chuỗi + logs -f --tail=100 (mục 8 PLAN)
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === tabId
          ? { ...t, status: "starting", error: null, options: { ...t.options, tail: 100 } }
          : t,
      ),
    }));
    try {
      await api.startLogStream(
        tabId,
        tab.ns,
        tab.pod,
        tab.container,
        { ...tab.options, tail: 100 },
      );
      set((s) => ({
        tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, status: "streaming" } : t)),
      }));
    } catch (e) {
      set((s) => ({
        tabs: s.tabs.map((t) =>
          t.id === tabId ? { ...t, status: "error", error: errMessage(e) } : t,
        ),
      }));
    }
  },

  clearTabLines: (tabId) => {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, lines: [] } : t)),
    }));
  },

  setTabAutoScroll: (tabId, v) => {
    set((s) => ({
      tabs: s.tabs.map((t) => (t.id === tabId ? { ...t, autoScroll: v } : t)),
    }));
  },

  setTabOptions: (tabId, opts) => {
    set((s) => ({
      tabs: s.tabs.map((t) =>
        t.id === tabId ? { ...t, options: { ...t.options, ...opts } } : t,
      ),
    }));
  },

  setManagerOpen: (open) => set({ managerOpen: open }),
}));

async function closeAllTabs(
  set: (partial: Partial<Store> | ((s: Store) => Partial<Store>)) => void,
  get: () => Store,
): Promise<void> {
  const { tabs } = get();
  for (const t of tabs) {
    try {
      await api.closeLogTab(t.id);
    } catch {
      /* ignore */
    }
    dropListeners("log:" + t.id);
    dropListeners("logend:" + t.id);
  }
  set({ tabs: [], activeTabId: null });
}
