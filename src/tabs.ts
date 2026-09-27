export interface TabInfo { label: string; title: string; }
export interface TabsSnapshot { tabs: TabInfo[]; active: string | null; revision: number; }

export function isTabsSnapshot(value: unknown): value is TabsSnapshot {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const snapshot = value as Record<string, unknown>;
  if (!Number.isSafeInteger(snapshot.revision) || (snapshot.revision as number) < 0 || !Array.isArray(snapshot.tabs)) return false;
  const labels = new Set<string>();
  for (const item of snapshot.tabs) {
    if (!item || typeof item !== "object" || Array.isArray(item)) return false;
    const tab = item as Record<string, unknown>;
    if (typeof tab.label !== "string" || !/^tab-(?:0|[1-9]\d{0,19})$/.test(tab.label) || labels.has(tab.label)) return false;
    if (typeof tab.title !== "string" || !tab.title.trim() || tab.title.length > 2048) return false;
    labels.add(tab.label);
  }
  return snapshot.active === null || (typeof snapshot.active === "string" && labels.has(snapshot.active));
}
