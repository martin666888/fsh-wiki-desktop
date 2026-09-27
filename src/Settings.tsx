import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { errorMessage } from "./feedback";
import {
  DEFAULT_PREFS, downloadPercent, isUpdateStatus, statusDetail, statusTitle,
  type UpdatePrefs, type UpdateStatus,
} from "./update";

interface FontConfig { enabled: boolean; en: string; cn: string; mono: string; }
interface Feedback { kind: "error" | "success" | "warning"; message: string; }
type TabKey = "font" | "about";
type UpdateCommand = "check_update" | "download_update" | "install_update";

const DEFAULT_CFG: FontConfig = {
  enabled: false, en: "Cascadia Mono", cn: "Microsoft YaHei", mono: "Cascadia Code",
};
const TABS: Array<{ key: TabKey; label: string }> = [
  { key: "font", label: "显示字体" }, { key: "about", label: "关于" },
];
const FONT_FIELDS = [
  { key: "en", label: "西文" }, { key: "cn", label: "中文" }, { key: "mono", label: "代码" },
] as const;

function Toggle({ checked, disabled, labelledBy, describedBy, onChange }: {
  checked: boolean;
  disabled: boolean;
  labelledBy: string;
  describedBy: string;
  onChange: (value: boolean) => void;
}) {
  return <button type="button" role="switch" aria-checked={checked}
    aria-labelledby={labelledBy} aria-describedby={describedBy} disabled={disabled}
    className={`tgl ${checked ? "on" : ""}`} onClick={() => onChange(!checked)}>
    <span className="tgl-knob" />
  </button>;
}

function FontSelect({ id, value, fonts, disabled, onChange }: {
  id: string; value: string; fonts: string[]; disabled: boolean;
  onChange: (value: string) => void;
}) {
  // Keep a saved font visible even when it is no longer installed.
  const options = fonts.includes(value) ? fonts : [value, ...fonts];
  return <select id={id} className="font-select" value={value} disabled={disabled}
    style={{ fontFamily: JSON.stringify(value) }} onChange={(event) => onChange(event.target.value)}>
    {options.map((font) => <option key={font} value={font}>{font}</option>)}
  </select>;
}

export default function Settings() {
  const [tab, setTab] = useState<TabKey>("font");
  const [fonts, setFonts] = useState<string[]>([]);
  const [cfg, setCfg] = useState<FontConfig>(DEFAULT_CFG);
  const [prefs, setPrefs] = useState<UpdatePrefs>(DEFAULT_PREFS);
  const [version, setVersion] = useState("");
  const [status, setStatus] = useState<UpdateStatus>({ kind: "idle" });
  const [fontReady, setFontReady] = useState(false);
  const [prefsReady, setPrefsReady] = useState(false);
  const [statusReady, setStatusReady] = useState(false);
  const [fontSaving, setFontSaving] = useState(false);
  const [prefsSaving, setPrefsSaving] = useState(false);
  const [updateCommand, setUpdateCommand] = useState<UpdateCommand | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [feedback, setFeedback] = useState<Feedback | null>(null);
  const [loadFailed, setLoadFailed] = useState(false);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const fontSaveLock = useRef(false);
  const prefsSaveLock = useRef(false);
  const updateLock = useRef(false);
  const cancelLock = useRef(false);
  const warningCount = useRef(0);
  const tabRefs = useRef<Record<TabKey, HTMLButtonElement | null>>({ font: null, about: null });

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const track = (unlisten: () => void) => {
      if (disposed) unlisten(); else unlisteners.push(unlisten);
    };
    const failed = (label: string, reason: unknown) => {
      if (disposed) return;
      setLoadFailed(true);
      setFeedback({ kind: "error", message: `${label}：${errorMessage(reason)}` });
    };
    setFontReady(false);
    setPrefsReady(false);
    setStatusReady(false);
    setLoadFailed(false);

    void (async () => {
      try {
        track(await listen<unknown>("settings-warning", (event) => {
          const payload = event.payload as { message?: unknown } | null;
          if (disposed || !payload || typeof payload.message !== "string" || payload.message.length > 16384) return;
          warningCount.current += 1;
          setFeedback({ kind: "warning", message: payload.message });
        }));
        if (disposed) return;
        const [availableFonts, config] = await Promise.all([
          invoke<string[]>("list_fonts"), invoke<FontConfig>("get_font_config"),
        ]);
        if (disposed) return;
        setFonts(availableFonts);
        setCfg({ ...config, en: config.en || DEFAULT_CFG.en, cn: config.cn || DEFAULT_CFG.cn, mono: config.mono || DEFAULT_CFG.mono });
        setFontReady(true);
      } catch (reason) { failed("无法读取字体设置", reason); }
    })();
    void invoke<UpdatePrefs>("get_update_prefs").then((value) => {
      if (!disposed) { setPrefs({ ...DEFAULT_PREFS, ...value }); setPrefsReady(true); }
    }).catch((reason) => failed("无法读取更新偏好", reason));
    void invoke<string>("app_version").then((value) => {
      if (!disposed) setVersion(value);
    }).catch((reason) => failed("无法读取版本", reason));

    // An event received while get_update_status is in flight is newer than its reply.
    void (async () => {
      let eventsReceived = 0;
      try {
        track(await listen<unknown>("update-status", (event) => {
          if (!isUpdateStatus(event.payload)) return;
          eventsReceived += 1;
          if (!disposed) { setStatus(event.payload); setStatusReady(true); }
        }));
        if (disposed) return;
        const seen = eventsReceived;
        const value = await invoke<unknown>("get_update_status");
        if (!isUpdateStatus(value)) throw new Error("更新状态格式无效，请重试");
        if (!disposed) {
          if (seen === eventsReceived) setStatus(value);
          setStatusReady(true);
        }
      } catch (reason) { failed("无法读取更新状态", reason); }
    })();
    void (async () => {
      let eventsReceived = 0;
      try {
        track(await listen<string>("settings-tab", (event) => {
          eventsReceived += 1;
          if (!disposed && (event.payload === "font" || event.payload === "about")) setTab(event.payload);
        }));
        if (disposed) return;
        const seen = eventsReceived;
        const value = await invoke<string | null>("take_settings_tab");
        if (!disposed && seen === eventsReceived && (value === "font" || value === "about")) setTab(value);
      } catch (reason) { failed("无法读取设置页面", reason); }
    })();
    return () => { disposed = true; unlisteners.forEach((unlisten) => unlisten()); };
  }, [loadAttempt]);

  const selectTab = (key: TabKey, focus = false) => {
    setTab(key);
    if (focus) tabRefs.current[key]?.focus();
  };
  const onTabKeyDown = (event: React.KeyboardEvent) => {
    const index = TABS.findIndex((item) => item.key === tab);
    let next: number;
    switch (event.key) {
      case "ArrowRight": next = (index + 1) % TABS.length; break;
      case "ArrowLeft": next = (index + TABS.length - 1) % TABS.length; break;
      case "Home": next = 0; break;
      case "End": next = TABS.length - 1; break;
      default: return;
    }
    event.preventDefault();
    selectTab(TABS[next].key, true);
  };

  const saveFont = async (next: FontConfig) => {
    if (!fontReady || fontSaveLock.current) return;
    fontSaveLock.current = true;
    setFontSaving(true);
    setFeedback(null);
    const warningsBeforeSave = warningCount.current;
    try {
      await invoke("set_font_config", { config: next });
      setCfg(next);
      if (warningCount.current === warningsBeforeSave) setFeedback({ kind: "success", message: "字体设置已保存" });
    } catch (reason) {
      setFeedback({ kind: "error", message: `字体设置未保存，已保留原设置：${errorMessage(reason)}` });
    } finally { fontSaveLock.current = false; setFontSaving(false); }
  };
  const savePrefs = async (next: UpdatePrefs) => {
    if (!prefsReady || prefsSaveLock.current) return;
    prefsSaveLock.current = true;
    setPrefsSaving(true);
    setFeedback(null);
    try {
      await invoke("set_update_prefs", { prefs: next });
      setPrefs(next);
      setFeedback({ kind: "success", message: "更新偏好已保存" });
    } catch (reason) {
      setFeedback({ kind: "error", message: `更新偏好未保存，已保留原设置：${errorMessage(reason)}` });
    } finally { prefsSaveLock.current = false; setPrefsSaving(false); }
  };
  const runUpdate = async (command: UpdateCommand) => {
    if (!statusReady || updateLock.current || status.kind === "installing") return;
    updateLock.current = true;
    setUpdateCommand(command);
    setFeedback(null);
    try { await invoke(command); }
    catch (reason) { setFeedback({ kind: "error", message: `更新操作未完成：${errorMessage(reason)}` }); }
    finally { updateLock.current = false; setUpdateCommand(null); }
  };
  const cancelUpdate = async () => {
    if (cancelLock.current) return;
    cancelLock.current = true;
    setCancelling(true);
    setFeedback(null);
    try {
      await invoke("cancel_update");
      setFeedback({ kind: "success", message: "更新操作已取消" });
    } catch (reason) { setFeedback({ kind: "error", message: `无法取消更新：${errorMessage(reason)}` }); }
    finally { cancelLock.current = false; setCancelling(false); }
  };

  const installing = status.kind === "installing" || updateCommand === "install_update";
  const percent = downloadPercent(status);
  const detail = statusDetail(status);
  const fontDisabled = !fontReady || fontSaving;
  const prefsDisabled = !prefsReady || prefsSaving || installing;
  const bodyChain = `${JSON.stringify(cfg.en)}, ${JSON.stringify(cfg.cn)}, sans-serif`;
  const monoChain = `${JSON.stringify(cfg.mono)}, Consolas, monospace`;
  const announcement = !statusReady ? "正在读取更新状态" :
    status.kind === "error" ? `更新出错：${status.message}` :
    status.kind === "downloaded" && status.notice ? `${statusTitle(status)}。${status.notice}` : statusTitle(status);

  const updateAction = () => {
    if (!statusReady) return <button type="button" className="btn" disabled>读取中…</button>;
    if (installing) return <button type="button" className="btn" disabled>正在安装…</button>;
    if (status.kind === "checking" || status.kind === "downloading") {
      return <button type="button" className="btn" disabled={cancelling} onClick={() => { void cancelUpdate(); }}>
        {cancelling ? "正在取消…" : "取消"}
      </button>;
    }
    if (status.kind === "available") {
      return <button type="button" className="btn primary" disabled={!!updateCommand} onClick={() => { void runUpdate("download_update"); }}>下载更新</button>;
    }
    if (status.kind === "downloaded") {
      return <button type="button" className="btn primary" disabled={!!updateCommand} onClick={() => { void runUpdate("install_update"); }}>重启并安装</button>;
    }
    return <button type="button" className="btn" disabled={!!updateCommand} onClick={() => { void runUpdate("check_update"); }}>检查更新</button>;
  };

  return (
    <div className="shell">
      <div className="stabs" role="tablist" aria-label="设置" onKeyDown={onTabKeyDown}>
        {TABS.map((item) => <button key={item.key} ref={(element) => { tabRefs.current[item.key] = element; }}
          type="button" role="tab" id={`stab-${item.key}`} aria-selected={tab === item.key}
          aria-controls={`spanel-${item.key}`} tabIndex={tab === item.key ? 0 : -1}
          className={`stab ${tab === item.key ? "on" : ""}`} onClick={() => selectTab(item.key)}>{item.label}</button>)}
      </div>
      <div className={`feedback ${feedback ? feedback.kind : ""}`}>
        <p role="alert" aria-atomic="true">{feedback?.kind === "error" ? feedback.message : ""}</p>
        <p role="status" aria-live="polite" aria-atomic="true">{feedback && feedback.kind !== "error" ? feedback.message : ""}</p>
        {loadFailed && <button type="button" className="link-btn" disabled={fontSaving || prefsSaving || !!updateCommand}
          onClick={() => { setFeedback(null); setLoadAttempt((attempt) => attempt + 1); }}>重新读取设置</button>}
      </div>
      <p className="sr-only" role="status" aria-live="polite" aria-atomic="true">{announcement}</p>

      <div className="pg" role="tabpanel" id="spanel-font" aria-labelledby="stab-font" hidden={tab !== "font"}>
        <div className="pg-head"><h1>显示字体</h1><p>仅影响本应用内的页面渲染，不修改文档内容</p></div>
        <div className="grp">
          <div className="row">
            <div className="row-txt"><span className="row-t" id="font-enabled-label">自定义显示字体</span><span className="row-d" id="font-enabled-description">使用下方字体渲染飞书页面</span></div>
            <Toggle checked={cfg.enabled} disabled={fontDisabled} labelledBy="font-enabled-label" describedBy="font-enabled-description"
              onChange={(enabled) => { void saveFont({ ...cfg, enabled }); }} />
          </div>
        </div>
        <div className={`grp ${cfg.enabled ? "" : "off"}`} aria-busy={fontSaving}>
          {FONT_FIELDS.map((field) => <div className="row" key={field.key}>
            <label className="row-t" htmlFor={`font-${field.key}`}>{field.label}</label>
            <FontSelect id={`font-${field.key}`} value={cfg[field.key]} fonts={fonts} disabled={fontDisabled || !cfg.enabled}
              onChange={(value) => { void saveFont({ ...cfg, [field.key]: value }); }} />
          </div>)}
          <div className="row prev-row"><div className="prev" aria-label="字体预览">
            <div className="prev-body" style={{ fontFamily: bodyChain }}>飞书文档 Feishu Docs 0123 ABCabc</div>
            <div className="prev-code" style={{ fontFamily: monoChain }}>let x = 42; // code</div>
          </div></div>
        </div>
        <div className="pg-foot">
          <span className="row-d" role="status">{fontSaving ? "正在保存…" : !fontReady ? "等待读取设置" : ""}</span>
          <button type="button" className="link-btn" disabled={fontDisabled} onClick={() => { void saveFont({ ...DEFAULT_CFG }); }}>恢复默认</button>
        </div>
      </div>

      <div className="pg" role="tabpanel" id="spanel-about" aria-labelledby="stab-about" hidden={tab !== "about"}>
        <div className="pg-head"><h1>关于</h1><p>飞书文档轻客户端</p></div>
        <div className="grp"><div className="row"><span className="row-t">当前版本</span><span className="row-v">{version || "…"}</span></div></div>
        <div className="grp">
          <div className="row update-row">
            <div className="row-txt"><span className="row-t">{statusReady ? statusTitle(status) : "正在读取更新状态…"}</span>{detail && <span className="row-d">{detail}</span>}</div>
            {updateAction()}
          </div>
          {status.kind === "downloading" && <div className="row prog-row">
            <div className={`prog ${percent === null ? "indeterminate" : ""}`} role="progressbar" aria-label="更新下载进度"
              aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent ?? undefined} aria-valuetext={percent === null ? "正在下载，尚无总大小" : `${percent}%`}>
              <div className="prog-bar" style={{ width: percent === null ? "30%" : `${percent}%` }} />
            </div>
          </div>}
        </div>
        <div className="grp" aria-busy={prefsSaving}>
          <div className="row">
            <div className="row-txt"><span className="row-t" id="auto-check-label">启动时检查更新</span><span className="row-d" id="auto-check-description">打开应用时在后台检查新版本</span></div>
            <Toggle checked={prefs.autoCheck} disabled={prefsDisabled} labelledBy="auto-check-label" describedBy="auto-check-description"
              onChange={(autoCheck) => { void savePrefs({ ...prefs, autoCheck }); }} />
          </div>
          <div className="row">
            <div className="row-txt"><span className="row-t" id="auto-download-label">发现新版本后自动下载</span><span className="row-d" id="auto-download-description">下载完成后仍由你点击安装</span></div>
            <Toggle checked={prefs.autoDownload} disabled={prefsDisabled} labelledBy="auto-download-label" describedBy="auto-download-description"
              onChange={(autoDownload) => { void savePrefs({ ...prefs, autoDownload }); }} />
          </div>
        </div>
      </div>
    </div>
  );
}
