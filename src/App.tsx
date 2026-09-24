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
      <div className="tabbar">
        {tabs.map((t) => (
          <div
            key={t.label}
            className={`tab ${t.label === active ? "active" : ""}`}
            onClick={() => activateTab(t.label)}
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
              ×
            </button>
          </div>
        ))}
        <button className="newtab" onClick={newTab} title="新建标签页">
          +
        </button>
        <button
          className="settings-btn"
          onClick={() => invoke("open_settings").catch(console.error)}
          title="显示字体设置"
        >
          ⚙
        </button>
      </div>
      {tabs.length === 0 && (
        <div className="empty">没有打开的标签页，点击 + 新建</div>
      )}
    </div>
  );
}
