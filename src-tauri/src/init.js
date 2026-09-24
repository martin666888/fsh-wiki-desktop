(() => {
  const internals = () => window.__TAURI_INTERNALS__;
  const currentLabel = () => {
    try {
      return internals().metadata.currentWebview.label;
    } catch (e) {
      return "";
    }
  };

  let lastTitle = "";
  const reportTitle = () => {
    const t = document.title || "";
    if (t && t !== lastTitle) {
      lastTitle = t;
      const it = internals();
      if (it && typeof it.invoke === "function") {
        try {
          it.invoke("plugin:event|emit", {
            event: "feishu-title",
            payload: { label: currentLabel(), title: t },
          });
        } catch (e) {}
      }
    }
  };
  reportTitle();
  window.addEventListener("DOMContentLoaded", reportTitle);
  setInterval(reportTitle, 1500);
})();
