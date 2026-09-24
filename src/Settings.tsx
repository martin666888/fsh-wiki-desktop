import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface FontConfig {
  enabled: boolean;
  en: string;
  cn: string;
  mono: string;
}

const DEFAULT_CFG: FontConfig = {
  enabled: false,
  en: "Cascadia Mono",
  cn: "Microsoft YaHei",
  mono: "Cascadia Code",
};

type TabKey = "font" | "about";

const TABS: Array<{ key: TabKey; label: string }> = [
  { key: "font", label: "显示字体" },
  { key: "about", label: "关于" },
];

function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      className={`tgl ${checked ? "on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="tgl-knob" />
    </button>
  );
}

function FontSelect({
  value,
  fonts,
  disabled,
  onChange,
}: {
  value: string;
  fonts: string[];
  disabled?: boolean;
  onChange: (v: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDoc);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <div
      ref={ref}
      className={`cbx ${open ? "open" : ""} ${disabled ? "dis" : ""}`}
    >
      <button
        type="button"
        className="cbx-btn"
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
      >
        <span className="cbx-val" style={{ fontFamily: `"${value}"` }}>
          {value || "默认"}
        </span>
        <svg className="cbx-chev" width="10" height="6" viewBox="0 0 10 6">
          <path
            d="M1 1 L5 5 L9 1"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.2"
          />
        </svg>
      </button>
      {open && (
        <div className="cbx-menu" role="listbox">
          {fonts.map((f) => (
            <div
              key={f}
              role="option"
              aria-selected={f === value}
              className={`cbx-opt ${f === value ? "sel" : ""}`}
              onClick={() => {
                onChange(f);
                setOpen(false);
              }}
            >
              <span className="cbx-opt-name" style={{ fontFamily: `"${f}"` }}>
                {f}
              </span>
              {f === value && <span className="cbx-tick">✓</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default function Settings() {
  const [tab, setTab] = useState<TabKey>("font");
  const [fonts, setFonts] = useState<string[]>([]);
  const [cfg, setCfg] = useState<FontConfig>(DEFAULT_CFG);
  const [version, setVersion] = useState("");

  const tabRefs = useRef<Record<TabKey, HTMLButtonElement | null>>({
    font: null,
    about: null,
  });

  useEffect(() => {
    invoke<string[]>("list_fonts")
      .then(setFonts)
      .catch(console.error);
    invoke<FontConfig>("get_font_config")
      .then((c) => setCfg({ ...DEFAULT_CFG, ...c }))
      .catch(console.error);
    invoke<string>("app_version")
      .then(setVersion)
      .catch(console.error);
  }, []);

  const selectTab = (key: TabKey, focus = false) => {
    setTab(key);
    if (focus) tabRefs.current[key]?.focus();
  };

  // 左右方向键在 tab 间移动（WAI-ARIA tabs 约定的键盘行为）
  const onTabKeyDown = (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const i = TABS.findIndex((t) => t.key === tab);
    const step = e.key === "ArrowRight" ? 1 : TABS.length - 1;
    const next = TABS[(i + step) % TABS.length];
    selectTab(next.key, true);
  };

  const save = (next: FontConfig) => {
    setCfg(next);
    invoke("set_font_config", { config: next }).catch(console.error);
  };

  const bodyChain = [cfg.en, cfg.cn].filter(Boolean).join(", ");
  const monoChain = [cfg.mono].filter(Boolean).join(", ");

  return (
    <div className="shell">
      <div className="stabs" role="tablist" onKeyDown={onTabKeyDown}>
        {TABS.map((t) => (
          <button
            key={t.key}
            ref={(el) => {
              tabRefs.current[t.key] = el;
            }}
            type="button"
            role="tab"
            id={`stab-${t.key}`}
            aria-selected={tab === t.key}
            aria-controls={`spanel-${t.key}`}
            tabIndex={tab === t.key ? 0 : -1}
            className={`stab ${tab === t.key ? "on" : ""}`}
            onClick={() => selectTab(t.key)}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === "font" ? (
        <div
          className="pg"
          role="tabpanel"
          id="spanel-font"
          aria-labelledby="stab-font"
        >
          <div className="pg-head">
            <h1>显示字体</h1>
            <p>仅影响本应用内的页面渲染，不修改文档内容</p>
          </div>

          <div className="grp">
            <div className="row">
              <div className="row-txt">
                <span className="row-t">自定义显示字体</span>
                <span className="row-d">使用下方字体渲染飞书页面</span>
              </div>
              <Toggle
                checked={cfg.enabled}
                onChange={(v) => save({ ...cfg, enabled: v })}
              />
            </div>
          </div>

          <div className={`grp ${cfg.enabled ? "" : "off"}`}>
            <div className="row">
              <span className="row-t">西文</span>
              <FontSelect
                value={cfg.en}
                fonts={fonts}
                disabled={!cfg.enabled}
                onChange={(v) => save({ ...cfg, en: v })}
              />
            </div>
            <div className="row">
              <span className="row-t">中文</span>
              <FontSelect
                value={cfg.cn}
                fonts={fonts}
                disabled={!cfg.enabled}
                onChange={(v) => save({ ...cfg, cn: v })}
              />
            </div>
            <div className="row">
              <span className="row-t">代码</span>
              <FontSelect
                value={cfg.mono}
                fonts={fonts}
                disabled={!cfg.enabled}
                onChange={(v) => save({ ...cfg, mono: v })}
              />
            </div>
            <div className="row prev-row">
              <div className="prev">
                <div className="prev-body" style={{ fontFamily: bodyChain }}>
                  飞书文档 Feishu Docs 0123 ABCabc
                </div>
                <div className="prev-code" style={{ fontFamily: monoChain }}>
                  let x = 42; // code
                </div>
              </div>
            </div>
          </div>

          <div className="pg-foot">
            <button
              className="link-btn"
              onClick={() => save({ ...DEFAULT_CFG, enabled: false })}
            >
              恢复默认
            </button>
          </div>
        </div>
      ) : (
        <div
          className="pg"
          role="tabpanel"
          id="spanel-about"
          aria-labelledby="stab-about"
        >
          <div className="grp">
            <div className="row">
              <span className="row-t">版本</span>
              <span className="row-v">{version || "读取中…"}</span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
