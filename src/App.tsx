import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { errorMessage } from "./feedback";
import { isUpdateStatus, needsAttention } from "./update";
import { EMPTY_SNAPSHOT, isSplit, isTabsSnapshot, type TabsSnapshot } from "./tabs";
import "./App.css";
import { DownloadNotice } from "./DownloadNotice";

const DEFAULT_URL = "https://www.feishu.cn/drive/home/";

interface BarError { message: string; retry?: () => void; }

export default function App() {
  const [snapshot, setSnapshot] = useState<TabsSnapshot>(EMPTY_SNAPSHOT);
  const active = snapshot.activeGroup;
  const tabs = snapshot.groups.map(group => {
    const layout = group.layout;
    const page = (label: string) => snapshot.tabs.find(tab => tab.label === label);
    const left = layout.mode === "single" ? page(layout.tab) : isSplit(layout) ? page(layout.left) : undefined;
    const right = layout.mode === "split" ? page(layout.right) : undefined;
    return { label: group.id, title: right ? `${left?.title} | ${right.title}` : left?.title || "新标签页",
      leftTitle: left?.title || "新标签页", rightTitle: right?.title,
      split: isSplit(layout), focused: isSplit(layout) ? layout.focused : "left",
      loading: left?.loading || right?.loading,
      error: left?.error || right?.error, errorLabel: left?.error ? left.label : right?.error ? right.label : null };
  });
  const latestRevision = useRef(-1);
  const [maximized, setMaximized] = useState(false);
  const [updateReady, setUpdateReady] = useState(false);
  const [error, setError] = useState<BarError | null>(null);
  const [notice, setNotice] = useState<{ message: string } | null>(null);
  const dismissNotice = useCallback(() => setNotice(null), []);
  const [connectionAttempt, setConnectionAttempt] = useState(0);
  const tabButtons = useRef(new Map<string, HTMLButtonElement>());
  const tabScroll = useRef<HTMLDivElement>(null);
  const focusAfterClose = useRef<string | null>(null);
  const newTabButton = useRef<HTMLButtonElement>(null);
  const splitLock = useRef(false);
  const [splitBusy, setSplitBusy] = useState(false);
  const split = isSplit(snapshot.layout);
  const toolbarTab = active ?? tabs[0]?.label;

  const reportError = useCallback((context: string, reason: unknown, retry?: () => void) => {
    setError({ message: `${context}：${errorMessage(reason)}`, retry });
  }, []);

  const perform = useCallback(async function run(context: string, operation: () => Promise<unknown>): Promise<boolean> {
    setError(null);
    try { await operation(); return true; }
    catch (reason) {
      reportError(context, reason, () => { void run(context, operation); });
      return false;
    }
  }, [reportError]);

  useEffect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    let unresize: (() => void) | undefined;
    const refresh = () => {
      void win.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      }).catch((reason) => {
        if (!disposed) reportError("无法读取窗口状态", reason);
      });
    };
    refresh();
    void win.onResized(refresh).then((unlisten) => {
      if (disposed) unlisten(); else unresize = unlisten;
    }).catch((reason) => {
      if (!disposed) reportError("无法监听窗口变化", reason);
    });
    return () => { disposed = true; unresize?.(); };
  }, [reportError]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const track = (unlisten: () => void) => {
      if (disposed) unlisten(); else unlisteners.push(unlisten);
    };
    const receiveTabs = (next: unknown) => {
      if (disposed || !isTabsSnapshot(next) || next.revision < latestRevision.current) return;
      latestRevision.current = next.revision;
      setSnapshot(next);
    };
    const retry = () => { setError(null); setConnectionAttempt((attempt) => attempt + 1); };

    // Subscribe before requesting the snapshot. Revision rejects a stale reply.
    void (async () => {
      try {
        track(await listen<unknown>("tabs-changed", (event) => receiveTabs(event.payload)));
        if (!disposed) {
          const next = await invoke<unknown>("list_tabs");
          if (!isTabsSnapshot(next)) throw new Error("标签页状态格式无效，请重试");
          receiveTabs(next);
        }
      } catch (reason) {
        if (!disposed) reportError("无法读取标签页", reason, retry);
      }
    })();
    void (async () => {
      let eventsReceived = 0;
      try {
        track(await listen<unknown>("update-status", (event) => {
          if (!isUpdateStatus(event.payload)) return;
          eventsReceived += 1;
          if (!disposed) setUpdateReady(needsAttention(event.payload));
        }));
        if (disposed) return;
        const seen = eventsReceived;
        const status = await invoke<unknown>("get_update_status");
        if (!isUpdateStatus(status)) throw new Error("更新状态格式无效，请重试");
        if (!disposed && seen === eventsReceived) setUpdateReady(needsAttention(status));
      } catch (reason) {
        if (!disposed) reportError("无法读取更新状态", reason, retry);
      }
    })();
    void listen<unknown>("ui-error", (event) => {
      const payload = event.payload as { message?: unknown } | null;
      if (!disposed && payload && typeof payload.message === "string" && payload.message.length <= 16384) setError({ message: payload.message });
    }).then(track).catch((reason) => {
      if (!disposed) reportError("无法监听操作提示", reason, retry);
    });
    void listen<unknown>("ui-notice", (event) => {
      const payload = event.payload as { message?: unknown } | null;
      if (!disposed && payload && typeof payload.message === "string" && payload.message.length <= 16384) {
        setNotice({ message: payload.message });
      }
    }).then(track).catch((reason) => {
      if (!disposed) reportError("无法监听下载提示", reason, retry);
    });
    return () => { disposed = true; unlisteners.forEach((unlisten) => unlisten()); };
  }, [connectionAttempt, reportError]);

  useEffect(() => {
    const button = active ? tabButtons.current.get(active) : undefined;
    button?.closest(".tab")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active, tabs.length]);

  useEffect(() => {
    if (focusAfterClose.current && !tabs.some((tab) => tab.label === focusAfterClose.current)) {
      const button = active ? tabButtons.current.get(active) : undefined;
      focusAfterClose.current = null;
      (button ?? newTabButton.current)?.focus();
    }
  }, [active, tabs]);

  useEffect(() => {
    const scroller = tabScroll.current;
    if (!scroller) return;
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || Math.abs(event.deltaX) >= Math.abs(event.deltaY) || scroller.scrollWidth <= scroller.clientWidth) return;
      event.preventDefault();
      const unit = event.deltaMode === 1 ? 24 : event.deltaMode === 2 ? scroller.clientWidth : 1;
      scroller.scrollLeft += event.deltaY * unit;
    };
    scroller.addEventListener("wheel", onWheel, { passive: false });
    return () => scroller.removeEventListener("wheel", onWheel);
  }, []);

  const newTab = () => { void perform("无法新建标签页", () => invoke("create_tab", { url: DEFAULT_URL })); };
  const activateTab = (label: string, keyboard = false) => {
    if (keyboard) tabButtons.current.get(label)?.focus();
    void perform("无法切换标签页", () => invoke("activate_tab", { label, focusContent: !keyboard }));
  };
  const closeTab = (label: string, keyboard = false) => {
    focusAfterClose.current = keyboard ? label : null;
    void perform("无法关闭标签页", () => invoke("close_tab", { label, focusContent: !keyboard })).then((ok) => {
      if (!ok) focusAfterClose.current = null;
    });
  };
  const navigateTabs = (event: React.KeyboardEvent, label: string) => {
    const index = tabs.findIndex((tab) => tab.label === label);
    let next: number;
    switch (event.key) {
      case "ArrowLeft": next = (index - 1 + tabs.length) % tabs.length; break;
      case "ArrowRight": next = (index + 1) % tabs.length; break;
      case "Home": next = 0; break;
      case "End": next = tabs.length - 1; break;
      case "Delete": event.preventDefault(); closeTab(label, true); return;
      default: return;
    }
    event.preventDefault();
    activateTab(tabs[next].label, true);
  };

  return (
    <div className="app">
      <div className="tabbar" data-tauri-drag-region
        onKeyDown={(event) => {
          if (!event.ctrlKey || event.altKey) return;
          if (event.key.toLowerCase() === "t") { event.preventDefault(); newTab(); }
          else if (event.key.toLowerCase() === "w" && active) { event.preventDefault(); closeTab(active, true); }
          else if (event.key === "Tab" && tabs.length) {
            event.preventDefault();
            const index = tabs.findIndex((tab) => tab.label === active);
            activateTab(tabs[(index + (event.shiftKey ? -1 : 1) + tabs.length) % tabs.length].label, true);
          }
        }}>
        <div className="tab-workspace">
          <div className="tab-scroll" ref={tabScroll} role="tablist" aria-label="打开的文档" aria-describedby="tab-help" data-tauri-drag-region>
            {tabs.map((tab) => (
              <div className={`tab ${tab.label === active ? "active" : ""} ${tab.split ? "split-tab" : ""}`} key={tab.label} role="presentation"
                onMouseDown={(event) => { if (event.button === 1) { event.preventDefault(); closeTab(tab.label); } }}>
                <button type="button" role="tab" className="tab-select" aria-selected={tab.label === active}
                  tabIndex={tab.label === toolbarTab && !error ? 0 : -1} title={tab.title} aria-label={`${tab.title}${tab.split ? "，分屏标签" : ""}${tab.error ? `，${tab.error}` : ""}`}
                  ref={(element) => { if (element) tabButtons.current.set(tab.label, element); else tabButtons.current.delete(tab.label); }}
                  onKeyDown={(event) => navigateTabs(event, tab.label)} onClick={() => activateTab(tab.label)}>
                  {tab.split && <svg className="tab-split-icon" aria-hidden="true" width="14" height="14" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.4"><rect x="2" y="3" width="16" height="14" rx="2"/><path d="M10 3v14"/></svg>}
                  <span className={`tab-title ${tab.focused === "left" ? "focused-title" : ""}`}>{tab.leftTitle}</span>
                  {tab.rightTitle && <><span className="tab-title-divider" aria-hidden="true"/><span className={`tab-title ${tab.focused === "right" ? "focused-title" : ""}`}>{tab.rightTitle}</span></>}
                  {tab.loading && <span className="tab-loading" aria-label="加载中"/>}
                </button>
                {tab.error && <button type="button" className="tab-close tab-load-error" title={`${tab.error}；点击重试`} aria-label="重试加载页面" onClick={() => { void perform("无法重试", () => invoke("reload_tab", { label: tab.errorLabel })); }}>↻</button>}
                <button type="button" className="tab-close" aria-label={`关闭 ${tab.title}`} title={`关闭 ${tab.title}`}
                  tabIndex={tab.label === active && !error ? 0 : -1} onClick={(event) => closeTab(tab.label, event.detail === 0)}>
                  <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12"><path d="M2 2 L10 10 M10 2 L2 10" stroke="currentColor" strokeWidth="1.3" /></svg>
                </button>
              </div>
            ))}
          </div>
          {error && <div className="bar-error">
            <span className="bar-error-text" role="alert" tabIndex={0} title={error.message}>{error.message}</span>
            {error.retry && <button type="button" onClick={error.retry}>重试</button>}
            <button type="button" aria-label="关闭提示" title="关闭提示" onClick={() => {
              setError(null);
              requestAnimationFrame(() => { (active ? tabButtons.current.get(active) : newTabButton.current)?.focus(); });
            }}>×</button>
          </div>}
        </div>
        <button type="button" className="newtab" ref={newTabButton} onClick={newTab} title="新建标签页" aria-label="新建标签页">
          <svg aria-hidden="true" width="12" height="12" viewBox="0 0 12 12"><path d="M6 1.5 V10.5 M1.5 6 H10.5" stroke="currentColor" strokeWidth="1.3" /></svg>
        </button>
        {notice && <DownloadNotice notice={notice} onDismiss={dismissNotice} onClose={() => {
          dismissNotice();
          (active ? tabButtons.current.get(active) : newTabButton.current)?.focus();
        }} />}
        <div className={`split-controls ${split ? "on" : ""}`}>
          <button type="button" className="split-toggle" aria-pressed={split} disabled={splitBusy || !tabs.length}
            title={split ? "退出分屏" : snapshot.viewport.canSplit ? "分屏浏览" : "分屏浏览（请先扩大窗口）"}
            aria-label={split ? "退出分屏" : "分屏浏览"}
            onClick={() => {
              if (splitLock.current) return;
              splitLock.current = true; setSplitBusy(true);
              void perform("无法切换分屏", () => invoke("set_split", { enabled: !split }))
                .finally(() => { splitLock.current = false; setSplitBusy(false); });
            }}>
            <svg aria-hidden="true" width="16" height="16" viewBox="0 0 20 20" fill="none" stroke="currentColor" strokeWidth="1.5"><rect x="2" y="3" width="16" height="14" rx="2"/><path d="M10 3v14"/></svg>
            <span>分屏</span>
          </button>

        </div>
        <button type="button" className={`settings-btn ${updateReady ? "dot" : ""}`}
          onClick={() => { void perform("无法打开设置", () => invoke("open_settings", { tab: updateReady ? "about" : "font" })); }}
          title={updateReady ? "设置：有更新待处理" : "设置"} aria-label={updateReady ? "设置：有更新待处理" : "设置"}>
          <svg aria-hidden="true" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
        <div className="wctrl">
          <button type="button" className="wbtn" title="最小化" aria-label="最小化" onClick={() => { void perform("无法最小化窗口", () => getCurrentWindow().minimize()); }}>
            <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10"><path d="M0 5 H10" stroke="currentColor" strokeWidth="1" /></svg>
          </button>
          <button type="button" className="wbtn" title={maximized ? "还原" : "最大化"} aria-label={maximized ? "还原" : "最大化"}
            onClick={() => { void perform("无法调整窗口", () => getCurrentWindow().toggleMaximize()); }}>
            {maximized ? <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10"><path d="M2.5 2.5 H9.5 V9.5 H2.5 Z M2.5 2.5 V0.5 H0.5 V7.5 H2.5" fill="none" stroke="currentColor" strokeWidth="1" /></svg>
              : <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10"><rect x="0.5" y="0.5" width="9" height="9" fill="none" stroke="currentColor" strokeWidth="1" /></svg>}
          </button>
          <button type="button" className="wbtn close" title="关闭" aria-label="关闭窗口" onClick={() => { void perform("无法关闭窗口", () => getCurrentWindow().close()); }}>
            <svg aria-hidden="true" width="10" height="10" viewBox="0 0 10 10"><path d="M0 0 L10 10 M10 0 L0 10" stroke="currentColor" strokeWidth="1" /></svg>
          </button>
        </div>
      </div>
      <p id="tab-help" className="sr-only">左右方向键、Home、End 切换标签；Delete 关闭标签；Enter 进入文档。标签栏内可使用 Ctrl+T 新建、Ctrl+W 关闭、Ctrl+Tab 切换。</p>
    </div>
  );
}
