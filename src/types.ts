export interface Profile {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  password: string;
  context: string;
  createdAt: string;
  updatedAt: string;
}

export interface ProfileInput {
  id?: string | null;
  name: string;
  host: string;
  port?: number | null;
  username?: string | null;
  password: string;
  context: string;
}

export interface PodRow {
  namespace: string;
  pod: string;
  ready: string;
  status: string;
  restarts: string;
  age: string;
}

export interface SearchResult {
  requestId: string;
  rows: PodRow[];
  truncated: boolean;
}

export interface LogOptions {
  tail: number;
  timestamps: boolean;
  since: string | null;
}

export interface TestResult {
  fingerprint: string;
  contexts: string[];
}

export interface AppErrorPayload {
  code: string;
  message: string;
  fingerprint?: string | null;
  expectedFingerprint?: string | null;
  extra?: string | null;
}

export type ConnState = "disconnected" | "connecting" | "ready" | "error";

export interface ConnPayload {
  state: ConnState;
  message: string | null;
}

export interface StoreStatus {
  initialized: boolean;
  kdf: number | null;
  locked: boolean;
  needPassphrase: boolean;
}

export type TabStatus =
  | "select-container"
  | "starting"
  | "streaming"
  | "ended"
  | "error";

export interface LogTab {
  id: string;
  ns: string;
  pod: string;
  containers: string[];
  container: string | null;
  status: TabStatus;
  error: string | null;
  options: LogOptions;
  lines: string[];
  autoScroll: boolean;
}

export function statusColor(status: string): string {
  const s = status.toUpperCase();
  if (s === "RUNNING") return "text-green-600 dark:text-green-400";
  if (s === "PENDING" || s === "CONTAINERCREATING") return "text-yellow-600 dark:text-yellow-400";
  if (
    s === "ERROR" ||
    s === "CRASHLOOPBACKOFF" ||
    s === "IMAGEPULLBACKOFF" ||
    s === "ERRIMAGEPULL" ||
    s === "EVICTED" ||
    s === "FAILED" ||
    s.startsWith("INIT:")
  )
    return "text-red-600 dark:text-red-400";
  if (s === "SUCCEEDED" || s === "COMPLETED") return "text-slate-500 dark:text-slate-400";
  return "text-sky-600 dark:text-sky-400";
}
