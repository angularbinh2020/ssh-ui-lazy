import { invoke } from "@tauri-apps/api/core";
import type {
  AppErrorPayload,
  LogOptions,
  Profile,
  ProfileInput,
  SearchResult,
  StoreStatus,
  TestResult,
} from "./types";

export class AppError extends Error implements AppErrorPayload {
  code: string;
  fingerprint?: string | null;
  expectedFingerprint?: string | null;
  extra?: string | null;

  constructor(payload: AppErrorPayload) {
    super(payload.message);
    this.code = payload.code;
    this.fingerprint = payload.fingerprint ?? null;
    this.expectedFingerprint = payload.expectedFingerprint ?? null;
    this.extra = payload.extra ?? null;
  }
}

function normalize(e: unknown): AppError {
  if (e && typeof e === "object" && "code" in e) {
    return new AppError(e as AppErrorPayload);
  }
  const message =
    typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
  return new AppError({ code: "OTHER", message });
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw normalize(e);
  }
}

export const api = {
  storeStatus: () => call<StoreStatus>("store_status"),
  storeUnlock: (passphrase: string | null) =>
    call<Profile[]>("store_unlock", { passphrase }),
  storeChangeKey: (newPassphrase: string | null) =>
    call<void>("store_change_key", { newPassphrase }),

  profileList: () => call<Profile[]>("profile_list"),
  profileSave: (profile: ProfileInput) =>
    call<Profile[]>("profile_save", { profile }),
  profileDelete: (id: string) => call<Profile[]>("profile_delete", { id }),
  profileDuplicate: (id: string) => call<Profile[]>("profile_duplicate", { id }),

  testConnection: (p: {
    host: string;
    port: number;
    username: string;
    password: string;
    trustNewHost: boolean;
  }) => call<TestResult>("test_connection", p),

  connect: (profileId: string, trustNewHost: boolean) =>
    call<void>("connect", { profileId, trustNewHost }),
  disconnect: () => call<void>("disconnect"),
  getContexts: () => call<string[]>("get_contexts"),

  searchPods: (requestId: string, keyword: string) =>
    call<SearchResult>("search_pods", { requestId, keyword }),

  openLogTab: (tabId: string, ns: string, pod: string) =>
    call<string[]>("open_log_tab", { tabId, ns, pod }),
  startLogStream: (
    tabId: string,
    ns: string,
    pod: string,
    container: string,
    options: LogOptions,
  ) =>
    call<void>("start_log_stream", {
      tabId,
      ns,
      pod,
      container,
      options: {
        tail: options.tail,
        timestamps: options.timestamps,
        since: options.since || null,
      },
    }),
  closeLogTab: (tabId: string) => call<void>("close_log_tab", { tabId }),
  saveLogFile: (path: string, content: string) =>
    call<void>("save_log_file", { path, content }),
};

export function errMessage(e: unknown): string {
  if (e instanceof AppError) return e.message;
  return normalize(e).message;
}
