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
  | { kind: "downloaded"; version: string }
  | { kind: "error"; message: string };

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
    s.kind === "available" || s.kind === "downloading" || s.kind === "downloaded"
  );
}

export function downloadPercent(s: UpdateStatus): number | null {
  if (s.kind !== "downloading" || !s.total) return null;
  return Math.min(100, Math.round((s.received / s.total) * 100));
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
      return pct === null ? "准备中…" : `${pct}%`;
    }
    case "downloaded":
      return "安装会关闭并重新打开本应用";
    case "error":
      return s.message;
    default:
      return null;
  }
}
