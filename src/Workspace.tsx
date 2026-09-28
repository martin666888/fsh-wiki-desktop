import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { errorMessage } from "./feedback";
import { isSplit, type TabInfo, type TabsSnapshot } from "./tabs";
import { useTabs } from "./useTabs";

function DocumentIcon() {
  return <svg aria-hidden="true" width="34" height="40" viewBox="0 0 34 40" fill="none">
    <path d="M7 2h13l8 8v25a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3V5a3 3 0 0 1 3-3Z" fill="currentColor" opacity=".09"/>
    <path d="M20 2H7a3 3 0 0 0-3 3v30a3 3 0 0 0 3 3h18a3 3 0 0 0 3-3V10L20 2Zm0 0v8h8M10 19h12M10 25h12M10 31h8" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"/>
  </svg>;
}

// Thumbnails stay in this local WebView's memory only, with one capture at a time.
// Native document instances are never activated or resized to obtain a preview.
const thumbnails = new Map<string, string | null>();
const pendingThumbnails = new Map<string, Promise<string | null>>();
let captureQueue: Promise<unknown> = Promise.resolve();
function thumbnail(tab: TabInfo): Promise<string | null> {
  const key = `${tab.label}:${tab.generation}`;
  if (thumbnails.has(key)) return Promise.resolve(thumbnails.get(key) ?? null);
  const pending = pendingThumbnails.get(key);
  if (pending) return pending;
  const job = captureQueue.then(async () => {
    try {
      const source = await invoke<string | null>("capture_tab_preview", { label: tab.label, generation: tab.generation });
      if (!source) return null;
      const image = new Image();
      image.src = source;
      await image.decode();
      const canvas = document.createElement("canvas");
      canvas.width = 480; canvas.height = 300;
      const context = canvas.getContext("2d");
      if (!context || !image.naturalWidth || !image.naturalHeight) return null;
      context.fillStyle = "#ffffff"; context.fillRect(0, 0, canvas.width, canvas.height);
      const scale = canvas.width / image.naturalWidth;
      context.drawImage(image, 0, 0, canvas.width, image.naturalHeight * scale);
      return canvas.toDataURL("image/webp", 0.72);
    } catch { return null; }
  }).then((result) => {
    pendingThumbnails.delete(key);
    for (const oldKey of thumbnails.keys()) if (oldKey.startsWith(`${tab.label}:`)) thumbnails.delete(oldKey);
    thumbnails.set(key, result);
    while (thumbnails.size > 24) thumbnails.delete(thumbnails.keys().next().value!);
    return result;
  });
  pendingThumbnails.set(key, job);
  captureQueue = job.catch(() => null);
  return job;
}

function Preview({ tab }: { tab: TabInfo }) {
  const element = useRef<HTMLSpanElement>(null);
  const [image, setImage] = useState<string | null>(null);
  useEffect(() => {
    setImage(null);
    if (tab.loading || tab.error || !element.current) return;
    let disposed = false;
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      observer.disconnect();
      void thumbnail(tab).then((value) => { if (!disposed) setImage(value); });
    }, { rootMargin: "80px" });
    observer.observe(element.current);
    return () => { disposed = true; observer.disconnect(); };
  }, [tab.label, tab.generation, tab.loading, tab.error]);
  return <span className={`card-preview ${image ? "has-image" : ""}`} ref={element}>
    {image ? <img src={image} alt="" draggable={false} /> : <><DocumentIcon /><span>{tab.loading ? "页面加载中" : "文档预览"}</span></>}
  </span>;
}

function Picker({ snapshot, onExit }: { snapshot: TabsSnapshot; onExit: () => void }) {
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [focusedCard, setFocusedCard] = useState<string | null>(null);
  const search = useRef<HTMLInputElement>(null);
  const cards = useRef(new Map<string, HTMLButtonElement>());
  const busyRef = useRef(false);
  const focusPending = useRef(false);
  const mounted = useRef(true);
  const root = useRef<HTMLDivElement>(null);
  const left = isSplit(snapshot.layout) ? snapshot.layout.left : "";
  const candidates = new Set(snapshot.groups.flatMap(group => group.layout.mode === "single" ? [group.layout.tab] : []));
  const available = snapshot.tabs.filter(tab => candidates.has(tab.label));
  const filtered = available.filter((tab) => tab.title.normalize("NFKC").toLocaleLowerCase().includes(query.trim().normalize("NFKC").toLocaleLowerCase()));
  const selectable = filtered.filter((tab) => tab.label !== left);
  const otherCount = available.length;
  const selectedCard = selectable.some((tab) => tab.label === focusedCard) ? focusedCard : selectable[0]?.label;

  useEffect(() => {
    mounted.current = true;
    let stop: (() => void) | undefined;
    let disposed = false;
    const frame = requestAnimationFrame(() => search.current?.focus());
    void listen<number>("focus-picker", () => search.current?.focus()).then((unlisten) => {
      if (disposed) unlisten(); else stop = unlisten;
    });
    return () => { mounted.current = false; disposed = true; cancelAnimationFrame(frame); stop?.(); };
  }, []);

  useEffect(() => {
    const labels = new Set(snapshot.tabs.map((tab) => tab.label));
    for (const key of thumbnails.keys()) if (!labels.has(key.split(":")[0])) thumbnails.delete(key);
    if (focusedCard && !labels.has(focusedCard)) {
      setFocusedCard(null);
      if (document.activeElement === document.body) search.current?.focus();
    }
  }, [snapshot.tabs, focusedCard]);

  const choose = async (tab: TabInfo) => {
    if (busyRef.current || tab.label === left) return;
    busyRef.current = true; setBusy(true); setError(null);
    try { await invoke("choose_split_tab", { label: tab.label, session: snapshot.pickerSession }); }
    catch (reason) { if (mounted.current) { setError(errorMessage(reason)); search.current?.focus(); } }
    finally { busyRef.current = false; if (mounted.current) setBusy(false); }
  };
  const create = async () => {
    if (busyRef.current) return;
    busyRef.current = true; setBusy(true); setError(null);
    try { await invoke("create_split_page", { session: snapshot.pickerSession }); }
    catch (reason) { if (mounted.current) setError(`无法在此打开首页：${errorMessage(reason)}`); }
    finally { busyRef.current = false; if (mounted.current) setBusy(false); }
  };
  const focusCard = (index: number) => {
    const tab = selectable[Math.max(0, Math.min(selectable.length - 1, index))];
    if (tab) { setFocusedCard(tab.label); cards.current.get(tab.label)?.focus(); }
  };
  const cardKey = (event: React.KeyboardEvent, label: string) => {
    const index = selectable.findIndex((tab) => tab.label === label);
    const first = selectable[0] && cards.current.get(selectable[0].label);
    const columns = first ? selectable.filter((tab) => cards.current.get(tab.label)?.offsetTop === first.offsetTop).length || 1 : 1;
    switch (event.key) {
      case "ArrowRight": event.preventDefault(); focusCard(index + 1); break;
      case "ArrowLeft": event.preventDefault(); focusCard(index - 1); break;
      case "ArrowDown": event.preventDefault(); focusCard(index + columns); break;
      case "ArrowUp": event.preventDefault(); if (index < columns) search.current?.focus(); else focusCard(index - columns); break;
      case "Home": event.preventDefault(); focusCard(0); break;
      case "End": event.preventDefault(); focusCard(selectable.length - 1); break;
    }
  };

  return <div className="picker" ref={root} aria-labelledby="picker-title" aria-busy={busy}
    onFocusCapture={() => {
      if (!isSplit(snapshot.layout) || snapshot.layout.focused === "right" || focusPending.current) return;
      focusPending.current = true;
      void invoke("focus_pane", { pane: "right", focusContent: false }).catch((reason) => {
        if (mounted.current) setError(errorMessage(reason));
      }).finally(() => { focusPending.current = false; });
    }}
    onKeyDown={(event) => {
      if (event.nativeEvent.isComposing) return;
      if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onExit(); }
    }}>
    <div className="picker-intro">
      <span className="eyebrow">分屏浏览</span>
      <h1 id="picker-title">选择另一篇文章</h1>
      <p>选择一个独立标签，与当前页面合并为分屏标签。</p>
    </div>
    <div className="search-field">
      <svg width="18" height="18" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><circle cx="8.5" cy="8.5" r="5.5"/><path d="m13 13 4 4"/></svg>
      <input ref={search} type="search" value={query} placeholder="搜索已打开的标签页" aria-label="搜索已打开的标签页" autoComplete="off" spellCheck={false}
        onChange={(event) => { setQuery(event.target.value); setFocusedCard(null); }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) return;
          if (event.key === "ArrowDown" && selectable.length) { event.preventDefault(); focusCard(0); }
          if (event.key === "Enter" && selectable.length === 1) { event.preventDefault(); void choose(selectable[0]); }
        }} />
      {query && <button type="button" aria-label="清空搜索" title="清空搜索" onClick={() => { setQuery(""); search.current?.focus(); }}>×</button>}
    </div>
    <p className="picker-error" role="alert">{error}</p>
    <div className="picker-list" aria-label="已打开的标签页">
      <div className="list-caption"><span>已打开的页面</span><span aria-live="polite">{selectable.length} 篇可选</span></div>
      {otherCount === 0 && <div className="picker-empty">
        <p>没有可加入的独立标签页。</p><span>打开飞书首页，找到想一起阅读的文章。</span>
        <button type="button" className="primary-button" disabled={busy} onClick={() => { void create(); }}>{busy ? "正在打开…" : "在此打开首页"}</button>
      </div>}
      {otherCount > 0 && selectable.length === 0 && <div className="picker-empty">
        <p>没有找到匹配的标签页</p><span>试试其他关键词，或清空搜索重新选择。</span>
        <button type="button" className="secondary-button" onClick={() => { setQuery(""); search.current?.focus(); }}>清空搜索</button>
      </div>}
      <div className="card-grid">
        {filtered.map((tab) => <button key={tab.label} type="button" className={`document-card ${tab.label === left ? "in-left" : ""}`}
          ref={(element) => { if (element) cards.current.set(tab.label, element); else cards.current.delete(tab.label); }}
          disabled={tab.label === left || busy} tabIndex={tab.label === selectedCard ? 0 : -1}
          title={tab.title} aria-label={`${tab.title}${tab.label === left ? "，正在左侧显示，不可重复选择" : "，在右侧打开"}`}
          onFocus={() => setFocusedCard(tab.label)} onKeyDown={(event) => cardKey(event, tab.label)} onClick={() => { void choose(tab); }}>
          <Preview tab={tab}/><span className="card-caption"><span className="card-title">{tab.title}</span>
            <span className="card-detail">{tab.label === left ? "正在左侧显示" : tab.error ? "页面加载失败 · 可选择后重试" : "在右侧阅读"}</span>
          </span>
        </button>)}
      </div>
    </div>
    <div className="picker-footer">
      <span>方向键选择 · Enter 打开 · Esc 退出分屏</span>
      {otherCount > 0 && <button type="button" className="text-button" disabled={busy} onClick={() => { void create(); }}>在此打开首页</button>}
    </div>
  </div>;
}

export default function Workspace() {
  const { snapshot, error: connectionError, retry } = useTabs();
  const { layout, viewport } = snapshot;
  const [error, setError] = useState<string | null>(null);
  const [dragging, setDragging] = useState(false);
  const dragLock = useRef(false);
  const dividerPress = useRef<{ time: number; x: number; y: number } | null>(null);
  const run = useCallback(async (command: string, args?: Record<string, unknown>) => {
    setError(null);
    try { await invoke(command, args); }
    catch (reason) { const message = errorMessage(reason); setError(message); void invoke("report_workspace_error", { message }).catch(() => {}); }
  }, []);
  const exit = () => { void run("set_split", { enabled: false }); };

  if (!isSplit(layout)) return <div className="workspace-idle">{layout.mode === "empty" && <div className="startup-message">
    <DocumentIcon/><p>{connectionError || "正在打开飞书文档…"}</p>
    {connectionError && <button type="button" className="secondary-button" onClick={retry}>重新连接</button>}
  </div>}</div>;

  if (viewport.suspended) return <div className="workspace suspended">
    <div className="narrow-notice" role="status"><span>{error || connectionError || "窗口较窄，暂时显示当前文章；扩大后恢复分屏。"}</span>
      <button type="button" className="header-action" onClick={() => { void getCurrentWindow().maximize().catch((reason) => setError(errorMessage(reason))); }}>扩大窗口</button>
      <button type="button" className="header-action" onClick={exit}>退出分屏</button>
    </div>
  </div>;

  return <main className={`workspace ${dragging ? "dragging" : ""}`} aria-label="分屏阅读工作区">
    <section className={`reading-pane ${layout.focused === "left" ? "current" : ""}`} style={{ left: 0, width: viewport.leftWidth }} aria-label="左侧文章">

    </section>
    <div className="divider" role="separator" tabIndex={0} aria-label="调整左右文章宽度" aria-orientation="vertical"
      aria-valuemin={Math.round(viewport.minRatio * 100)} aria-valuemax={Math.round((1 - viewport.minRatio) * 100)}
      aria-valuenow={Math.round(viewport.leftWidth / Math.max(1, viewport.width - 6) * 100)} aria-valuetext={`左侧 ${Math.round(viewport.leftWidth / Math.max(1, viewport.width - 6) * 100)}%`}
      title="拖动调整宽度，双击恢复均分；方向键微调，Enter 均分" style={{ left: viewport.leftWidth }}
      onPointerDown={(event) => {
        if (event.button !== 0 || dragLock.current) return;
        event.preventDefault(); event.currentTarget.focus();
        // Native capture receives mouse-up, so the WebView may never emit dblclick.
        const previous = dividerPress.current;
        dividerPress.current = { time: event.timeStamp, x: event.clientX, y: event.clientY };
        if (previous && event.timeStamp - previous.time < 500
          && Math.abs(event.clientX - previous.x) < 4 && Math.abs(event.clientY - previous.y) < 4) {
          dividerPress.current = null;
          void run("set_split_ratio", { ratio: 0.5, group: snapshot.activeGroup });
          return;
        }
        dragLock.current = true; setDragging(true);
        void run("begin_split_resize").finally(() => { dragLock.current = false; setDragging(false); });
      }}
      onDoubleClick={() => { void run("set_split_ratio", { ratio: 0.5, group: snapshot.activeGroup }); }}
      onKeyDown={(event) => {
        const current = viewport.leftWidth / Math.max(1, viewport.width - 6);
        let ratio: number;
        switch (event.key) {
          case "ArrowLeft": ratio = current - (event.shiftKey ? 0.1 : 0.02); break;
          case "ArrowRight": ratio = current + (event.shiftKey ? 0.1 : 0.02); break;
          case "Home": ratio = viewport.minRatio; break;
          case "End": ratio = 1 - viewport.minRatio; break;
          case "Enter": ratio = 0.5; break;
          default: return;
        }
        event.preventDefault(); void run("set_split_ratio", { ratio, group: snapshot.activeGroup });
      }}><span/></div>
    <section className={`reading-pane ${layout.focused === "right" ? "current" : ""}`} style={{ left: viewport.rightX, right: 0 }} aria-label={layout.mode === "picking" ? "选择右侧文章" : "右侧文章"}>

      {layout.mode === "picking" && <Picker key={snapshot.pickerSession} snapshot={snapshot} onExit={exit}/>}
    </section>
    <span className="sr-only" role="status" aria-live="polite">{error || connectionError || ""}</span>
  </main>;
}
