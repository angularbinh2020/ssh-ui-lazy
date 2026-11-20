import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../api", () => ({
  api: {
    openLogTab: vi.fn(async () => ["main", "sidecar"]),
    startLogStream: vi.fn(async () => undefined),
    closeLogTab: vi.fn(async () => undefined),
    searchPods: vi.fn(async () => ({ rows: [], truncated: false })),
  },
  errMessage: (e: unknown) => (e instanceof Error ? e.message : String(e)),
}));

const eventHandlers: Record<string, (e: { payload: unknown }) => void> = {};

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
    eventHandlers[event] = cb;
    return () => {};
  }),
}));

import { api } from "../api";
import { useStore } from "./useStore";

const initialState = useStore.getState();

function emit(event: string, payload: unknown): void {
  const handler = eventHandlers[event];
  if (!handler) throw new Error(`No handler for event ${event}`);
  handler({ payload });
}

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  document.documentElement.className = "";
  useStore.setState(initialState, true);
});

describe("theme", () => {
  it("setTheme lưu localStorage và cập nhật class dark", () => {
    useStore.getState().setTheme("light");

    expect(useStore.getState().theme).toBe("light");
    expect(localStorage.getItem("theme")).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
  });

  it("toggleTheme chuyển dark ↔ light", () => {
    useStore.getState().toggleTheme();
    expect(useStore.getState().theme).toBe("light");

    useStore.getState().toggleTheme();
    expect(useStore.getState().theme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
  });
});

describe("search state", () => {
  it("setKeyword cập nhật keyword", () => {
    useStore.getState().setKeyword("nginx");
    expect(useStore.getState().keyword).toBe("nginx");
  });

  it("runSearch reset kết quả khi keyword < 3 ký tự", async () => {
    useStore.setState({
      activeProfileId: "p1",
      conn: { state: "ready", message: null },
    });
    useStore.getState().setKeyword("ab");

    await useStore.getState().runSearch();

    expect(api.searchPods).not.toHaveBeenCalled();
    expect(useStore.getState().search.loading).toBe(false);
    expect(useStore.getState().search.rows).toEqual([]);
  });
});

describe("tabs", () => {
  it("openTab tạo tab mới và chọn container duy nhất tự động", async () => {
    vi.mocked(api.openLogTab).mockResolvedValueOnce(["main"]);

    await useStore.getState().openTab("kube-system", "nginx-1");

    const { tabs, activeTabId } = useStore.getState();
    expect(tabs).toHaveLength(1);
    expect(tabs[0]).toMatchObject({
      ns: "kube-system",
      pod: "nginx-1",
      containers: ["main"],
      container: "main",
      status: "streaming",
    });
    expect(activeTabId).toBe(tabs[0]!.id);
    expect(api.startLogStream).toHaveBeenCalledWith(
      tabs[0]!.id,
      "kube-system",
      "nginx-1",
      "main",
      tabs[0]!.options,
    );
  });

  it("openTab với pod đã mở chỉ kích hoạt tab cũ", async () => {
    await useStore.getState().openTab("kube-system", "nginx-1");
    const firstTabs = useStore.getState().tabs;

    await useStore.getState().openTab("kube-system", "nginx-1");

    expect(useStore.getState().tabs).toBe(firstTabs);
    expect(firstTabs).toHaveLength(1);
  });

  it("openTab lỗi khi lấy containers → tab status error", async () => {
    vi.mocked(api.openLogTab).mockRejectedValueOnce(new Error("pod not found"));

    await useStore.getState().openTab("default", "api-0");

    const tab = useStore.getState().tabs[0]!;
    expect(tab.status).toBe("error");
    expect(tab.error).toBe("pod not found");
  });

  it("log event nối dòng và cắt ring buffer ở 10_000 dòng", async () => {
    await useStore.getState().openTab("default", "api-0");
    const tabId = useStore.getState().tabs[0]!.id;
    const lines = Array.from({ length: 10_005 }, (_, i) => `line-${i}`);

    emit(`log:${tabId}`, { lines });

    const tab = useStore.getState().tabs[0]!;
    expect(tab.lines).toHaveLength(10_000);
    expect(tab.lines[0]).toBe("line-5");
    expect(tab.lines[tab.lines.length - 1]).toBe("line-10004");
  });

  it("chooseContainer lỗi → tab status error với message", async () => {
    vi.mocked(api.startLogStream).mockRejectedValueOnce(
      new Error("container crash"),
    );
    await useStore.getState().openTab("default", "api-0");
    const tabId = useStore.getState().tabs[0]!.id;

    await useStore.getState().chooseContainer(tabId, "main");

    const tab = useStore.getState().tabs[0]!;
    expect(tab.status).toBe("error");
    expect(tab.error).toBe("container crash");
  });

  it("closeTab xóa tab và chuyển activeTabId về tab cuối còn lại", async () => {
    await useStore.getState().openTab("default", "pod-a");
    await useStore.getState().openTab("default", "pod-b");
    const [tabA, tabB] = useStore.getState().tabs;

    await useStore.getState().closeTab(tabB!.id);

    const { tabs, activeTabId } = useStore.getState();
    expect(tabs).toHaveLength(1);
    expect(tabs[0]!.id).toBe(tabA!.id);
    expect(activeTabId).toBe(tabA!.id);
    expect(api.closeLogTab).toHaveBeenCalledWith(tabB!.id);
  });

  it("clearTabLines xóa toàn bộ dòng của tab", async () => {
    await useStore.getState().openTab("default", "pod-a");
    const tabId = useStore.getState().tabs[0]!.id;
    emit(`log:${tabId}`, { lines: ["a", "b"] });

    useStore.getState().clearTabLines(tabId);

    expect(useStore.getState().tabs[0]!.lines).toEqual([]);
  });

  it("setTabOptions merge options giữ giá trị cũ", async () => {
    await useStore.getState().openTab("default", "pod-a");
    const tabId = useStore.getState().tabs[0]!.id;

    useStore.getState().setTabOptions(tabId, { timestamps: true });

    expect(useStore.getState().tabs[0]!.options).toEqual({
      tail: 500,
      timestamps: true,
      since: null,
    });
  });
});
