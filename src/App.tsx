import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";

const DEFAULT_URL = "https://www.feishu.cn/drive/home/";

interface TabInfo {
  label: string;
  title: string;
}

export default function App() {
  const [tabs, setTabs] = useState<TabInfo[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    win.isMaximized().then(setMaximized).catch(() => {});
    let disposed = false;
    let unresize: (() => void) | null = null;
    win
      .onResized(() => {
        if (!disposed) win.isMaximized().then(setMaximized).catch(() => {});
      })
      .then((un) => {
        if (disposed) un();
        else unresize = un;
      });
    return () => {
      disposed = true;
      unresize?.();
    };
  }, []);

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    let disposed = false;
    const track = (p: Promise<() => void>) =>
      p.then((fn) => {
        if (disposed) fn();
        else unlisteners.push(fn);
      });

    track(
      listen<{ label: string }>("tab-created", (e) => {
        setTabs((ts) =>
          ts.some((t) => t.label === e.payload.label)
            ? ts
            : [...ts, { label: e.payload.label, title: "新标签页" }],
        );
        setActive(e.payload.label);
      }),
    );
    track(
      listen<string>("tab-closed", (e) => {
        setTabs((ts) => ts.filter((t) => t.label !== e.payload));
      }),
    );
    track(
      listen<string>("tab-activated", (e) => {
        setActive(e.payload);
      }),
    );
    track(
      listen<{ label: string; title: string }>("feishu-title", (e) => {
        setTabs((ts) =>
          ts.map((t) =>
            t.label === e.payload.label ? { ...t, title: e.payload.title } : t,
          ),
        );
      }),
    );

    invoke<{ tabs: string[]; active: string | null }>("list_tabs")
      .then((snap) => {
        setTabs((ts) =>
          snap.tabs.map(
            (l) => ts.find((t) => t.label === l) ?? { label: l, title: "新标签页" },
          ),
        );
        if (snap.active) setActive(snap.active);
      })
      .catch(console.error);

    return () => {
      disposed = true;
      unlisteners.forEach((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const t = tabs.find((x) => x.label === active);
    if (t) {
      getCurrentWindow()
        .setTitle(`${t.title} - 飞书文档`)
        .catch(() => {});
    }
  }, [tabs, active]);

  const newTab = useCallback(() => {
    invoke("create_tab", { url: DEFAULT_URL }).catch(console.error);
  }, []);

  const closeTab = useCallback((label: string) => {
    invoke("close_tab", { label }).catch(console.error);
  }, []);

  const activateTab = useCallback((label: string) => {
    setActive(label);
    invoke("activate_tab", { label }).catch(console.error);
  }, []);

  return (
    <div className="app">
      <div
        className="tabbar"
        data-tauri-drag-region
        onDoubleClick={(e) => {
          if (e.target === e.currentTarget) {
            getCurrentWindow().toggleMaximize().catch(() => {});
          }
        }}
      >
        {tabs.map((t) => (
          <div
            key={t.label}
            className={`tab ${t.label === active ? "active" : ""}`}
            onClick={() => activateTab(t.label)}
            onMouseDown={(e) => {
              if (e.button === 1) {
                e.preventDefault();
                closeTab(t.label);
              }
            }}
            title={t.title}
          >
            <span className="tab-title">{t.title}</span>
            <button
              className="tab-close"
              onClick={(e) => {
                e.stopPropagation();
                closeTab(t.label);
              }}
            >
              <svg width="12" height="12" viewBox="0 0 12 12">
                <path
                  d="M2 2 L10 10 M10 2 L2 10"
                  stroke="currentColor"
                  strokeWidth="1.3"
                />
              </svg>
            </button>
          </div>
        ))}
        <button className="newtab" onClick={newTab} title="新建标签页">
          <svg width="12" height="12" viewBox="0 0 12 12">
            <path
              d="M6 1.5 V10.5 M1.5 6 H10.5"
              stroke="currentColor"
              strokeWidth="1.3"
            />
          </svg>
        </button>
        <button
          className="settings-btn"
          onClick={() => invoke("open_settings").catch(console.error)}
          title="显示字体设置"
        >
          <svg
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
          >
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
        <div className="wctrl">
          <button
            className="wbtn"
            title="最小化"
            onClick={() => getCurrentWindow().minimize().catch(() => {})}
          >
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M0 5 H10" stroke="currentColor" strokeWidth="1" />
            </svg>
          </button>
          <button
            className="wbtn"
            title={maximized ? "还原" : "最大化"}
            onClick={() => getCurrentWindow().toggleMaximize().catch(() => {})}
          >
            {maximized ? (
              <svg width="10" height="10" viewBox="0 0 10 10">
                <path
                  d="M2.5 2.5 H9.5 V9.5 H2.5 Z M2.5 2.5 V0.5 H0.5 V7.5 H2.5"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1"
                />
              </svg>
            ) : (
              <svg width="10" height="10" viewBox="0 0 10 10">
                <rect
                  x="0.5"
                  y="0.5"
                  width="9"
                  height="9"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1"
                />
              </svg>
            )}
          </button>
          <button
            className="wbtn close"
            title="关闭"
            onClick={() => getCurrentWindow().close().catch(() => {})}
          >
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path
                d="M0 0 L10 10 M10 0 L0 10"
                stroke="currentColor"
                strokeWidth="1"
              />
            </svg>
          </button>
        </div>
      </div>
      {tabs.length === 0 && (
        <div className="empty">没有打开的标签页，点击 + 新建</div>
      )}
    </div>
  );
}
