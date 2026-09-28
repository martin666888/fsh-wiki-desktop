export type Pane = "left" | "right";
export type WorkspaceLayout =
  | { mode: "empty" }
  | { mode: "single"; tab: string }
  | { mode: "picking"; left: string; focused: Pane; ratio: number }
  | { mode: "split"; left: string; right: string; focused: Pane; ratio: number };
export interface TabInfo { label: string; title: string; loading: boolean; error: string | null; generation: number; }
export interface Viewport {
  width: number; height: number; leftWidth: number; rightX: number;
  canSplit: boolean; suspended: boolean; minRatio: number;
}
export interface TabGroup { id: string; layout: WorkspaceLayout; }
export interface TabsSnapshot {
  groups: TabGroup[]; activeGroup: string | null;
  tabs: TabInfo[]; active: string | null; revision: number;
  layout: WorkspaceLayout; viewport: Viewport; pickerSession: number;
}
export const EMPTY_SNAPSHOT: TabsSnapshot = {
  groups: [], activeGroup: null, tabs: [], active: null, revision: -1, layout: { mode: "empty" }, pickerSession: 0,
  viewport: { width: 0, height: 0, leftWidth: 0, rightX: 0, canSplit: false, suspended: false, minRatio: 0.5 },
};
export function isSplit(layout: WorkspaceLayout): layout is Extract<WorkspaceLayout, { mode: "picking" | "split" }> {
  return layout.mode === "picking" || layout.mode === "split";
}
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
    if (typeof tab.loading !== "boolean" || (tab.error !== null && typeof tab.error !== "string") || !Number.isSafeInteger(tab.generation) || (tab.generation as number) < 0) return false;
    labels.add(tab.label);
  }
  if (snapshot.active !== null && (typeof snapshot.active !== "string" || !labels.has(snapshot.active))) return false;
  if (!Number.isSafeInteger(snapshot.pickerSession) || (snapshot.pickerSession as number) < 0) return false;
  if (!Array.isArray(snapshot.groups)) return false;
  const groupIds = new Set<string>();
  const owned = new Set<string>();
  for (const raw of snapshot.groups) {
    if (!raw || typeof raw !== "object") return false;
    const group = raw as TabGroup;
    if (typeof group.id !== "string" || groupIds.has(group.id) || !group.layout) return false;
    groupIds.add(group.id);
    const state = group.layout;
    if (state.mode === "empty") return false;
    if (state.mode !== "single" && state.mode !== "picking" && state.mode !== "split") return false;
    if (isSplit(state) && (!Number.isFinite(state.ratio) || state.ratio < 0 || state.ratio > 1 || !["left", "right"].includes(state.focused))) return false;
    const pages = state.mode === "single" ? [state.tab] : state.mode === "picking" ? [state.left] : [state.left, state.right];
    for (const page of pages) {
      if (!labels.has(page) || owned.has(page)) return false;
      owned.add(page);
    }
  }
  if (snapshot.activeGroup !== null && (typeof snapshot.activeGroup !== "string" || !groupIds.has(snapshot.activeGroup))) return false;
  if (snapshot.groups.length && snapshot.activeGroup === null) return false;
  const selected = (snapshot.groups as TabGroup[]).find(g => g.id === snapshot.activeGroup);
  if (selected && JSON.stringify(selected.layout) !== JSON.stringify(snapshot.layout)) return false;
  const layout = snapshot.layout as Record<string, unknown> | null;
  const viewport = snapshot.viewport as Record<string, unknown> | null;
  if (!layout || !viewport) return false;
  for (const key of ["width", "height", "leftWidth", "rightX", "minRatio"]) {
    if (typeof viewport[key] !== "number" || !Number.isFinite(viewport[key]) || (viewport[key] as number) < 0) return false;
  }
  if (typeof viewport.canSplit !== "boolean" || typeof viewport.suspended !== "boolean" || (viewport.minRatio as number) > 0.5) return false;
  switch (layout.mode) {
    case "empty": return snapshot.active === null;
    case "single": return typeof layout.tab === "string" && labels.has(layout.tab) && snapshot.active === layout.tab;
    case "picking":
    case "split": {
      if (typeof layout.left !== "string" || !labels.has(layout.left) || (layout.focused !== "left" && layout.focused !== "right")) return false;
      if (typeof layout.ratio !== "number" || !Number.isFinite(layout.ratio) || layout.ratio < 0 || layout.ratio > 1) return false;
      if (layout.mode === "split" && (typeof layout.right !== "string" || !labels.has(layout.right) || layout.left === layout.right)) return false;
      return snapshot.active === (layout.focused === "left" ? layout.left : layout.mode === "split" ? layout.right : null);
    }
    default: return false;
  }
}
