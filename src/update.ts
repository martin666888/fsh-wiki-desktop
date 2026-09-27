/** 与 Rust 侧 UpdateStatus 对应（serde tag = "kind"） */
export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate"; checked_at: number }
  | { kind: "available"; version: string }
  | {
      kind: "downloading";
      version: string;
      received: number;
      total: number | null;
    }
  | { kind: "downloaded"; version: string; notice?: string }
  | { kind: "installing"; version: string }
  | { kind: "error"; message: string };

export function isUpdateStatus(value: unknown): value is UpdateStatus {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const status = value as Record<string, unknown>;
  const text = (value: unknown, max: number) => typeof value === "string" && value.length > 0 && value.length <= max;
  const count = (value: unknown) => Number.isSafeInteger(value) && (value as number) >= 0;
  switch (status.kind) {
    case "idle":
    case "checking": return true;
    case "upToDate": return count(status.checked_at);
    case "available":
    case "installing": return text(status.version, 128);
    case "downloaded": return text(status.version, 128) && (status.notice === undefined || text(status.notice, 16384));
    case "downloading": return text(status.version, 128) && count(status.received) && (status.total === null || count(status.total));
    case "error": return text(status.message, 16384);
    default: return false;
  }
}

export interface UpdatePrefs {
  autoCheck: boolean;
  autoDownload: boolean;
}

export const DEFAULT_PREFS: UpdatePrefs = {
  autoCheck: true,
  autoDownload: false,
};

/** 有没处理完的更新，界面上需要提醒用户 */
export function needsAttention(s: UpdateStatus): boolean {
  return (
    s.kind === "available" || s.kind === "downloading" || s.kind === "downloaded" || s.kind === "installing"
  );
}

export function downloadPercent(s: UpdateStatus): number | null {
  if (s.kind !== "downloading" || !s.total) return null;
  return Math.max(0, Math.min(100, Math.round((s.received / s.total) * 100)));
}

export function statusTitle(s: UpdateStatus): string {
  switch (s.kind) {
    case "idle":
      return "尚未检查更新";
    case "checking":
      return "正在检查更新…";
    case "upToDate":
      return "已是最新版本";
    case "available":
      return `发现新版本 ${s.version}`;
    case "downloading":
      return `正在下载 ${s.version}`;
    case "downloaded":
      return `${s.version} 已下载完成`;
    case "installing":
      return `正在安装 ${s.version}`;
    case "error":
      return "更新出错";
  }
}

export function statusDetail(s: UpdateStatus): string | null {
  switch (s.kind) {
    case "upToDate":
      return `上次检查 ${new Date(s.checked_at).toLocaleTimeString()}`;
    case "available":
      return "下载后由你决定什么时候安装";
    case "downloading": {
      const pct = downloadPercent(s);
      if (pct !== null) return `${pct}%`;
      if (s.received === 0) return "正在连接…";
      const size = s.received >= 1024 * 1024
        ? `${(s.received / (1024 * 1024)).toFixed(1)} MB`
        : s.received >= 1024 ? `${(s.received / 1024).toFixed(1)} KB` : `${s.received} B`;
      return `已下载 ${size}（总大小未知）`;
    }
    case "downloaded":
      return s.notice || "安装会关闭并重新打开本应用";
    case "installing":
      return "请稍候，应用将自动重新启动";
    case "error":
      return s.message;
    default:
      return null;
  }
}
