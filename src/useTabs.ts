import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { EMPTY_SNAPSHOT, isTabsSnapshot } from "./tabs";
import { errorMessage } from "./feedback";

export function useTabs() {
  const [snapshot, setSnapshot] = useState(EMPTY_SNAPSHOT);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  const revision = useRef(-1);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const accept = (value: unknown) => {
      if (!disposed && isTabsSnapshot(value) && value.revision >= revision.current) {
        revision.current = value.revision;
        setSnapshot(value);
      }
    };
    void (async () => {
      try {
        const stop = await listen<unknown>("tabs-changed", (event) => accept(event.payload));
        if (disposed) { stop(); return; }
        unlisten = stop;
        const value = await invoke<unknown>("list_tabs");
        if (!isTabsSnapshot(value)) throw new Error("工作区状态格式无效");
        accept(value);
        if (!disposed) setError(null);
      } catch (reason) { if (!disposed) setError(errorMessage(reason)); }
    })();
    return () => { disposed = true; unlisten?.(); };
  }, [attempt]);
  return { snapshot, error, retry: () => setAttempt((value) => value + 1) };
}
