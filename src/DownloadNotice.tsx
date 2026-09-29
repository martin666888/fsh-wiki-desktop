import { useEffect, useState } from "react";

interface DownloadNoticeProps {
  notice: { message: string };
  onDismiss: () => void;
  onClose: () => void;
}

export function DownloadNotice({ notice, onDismiss, onClose }: DownloadNoticeProps) {
  const [hovered, setHovered] = useState(false);
  const [focused, setFocused] = useState(false);
  const [fading, setFading] = useState(false);

  useEffect(() => {
    setFading(false);
    if (hovered || focused) return;

    const fadeTimer = window.setTimeout(() => setFading(true), 5000);
    const dismissTimer = window.setTimeout(onDismiss, 5250);
    return () => {
      window.clearTimeout(fadeTimer);
      window.clearTimeout(dismissTimer);
    };
  }, [notice, hovered, focused, onDismiss]);

  return (
    <div className={`download-notice${fading ? " fading" : ""}`}
      onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)}
      onFocus={() => setFocused(true)}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false);
      }}>
      <span className="download-notice-text" role="status" tabIndex={0} title={notice.message}>
        {notice.message}
      </span>
      <button type="button" aria-label="关闭下载提示" title="关闭" onClick={onClose}>×</button>
    </div>
  );
}
