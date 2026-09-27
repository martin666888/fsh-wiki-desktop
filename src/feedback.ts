export function errorMessage(reason: unknown): string {
  if (reason instanceof Error) return reason.message;
  if (typeof reason === "string") return reason;
  try { return JSON.stringify(reason) || "请稍后重试"; }
  catch { return "请稍后重试"; }
}
