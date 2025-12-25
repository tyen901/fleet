import { useEffect, useRef, useState } from "react";
import { getSyncLogs, type LogEntry } from "../../api";

export function useSyncLog(isEnabled: boolean) {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const cursorRef = useRef(0);

  useEffect(() => {
    if (!isEnabled) return;

    const poll = async () => {
      try {
        const page = await getSyncLogs(cursorRef.current);
        if (page.entries.length > 0) {
          cursorRef.current = page.next_cursor;
          setEntries((prev) => [...prev, ...page.entries].slice(-1000));
        }
      } catch (e) {
        console.error("Log fetch error:", e);
      }
    };

    const interval = setInterval(poll, 500);
    void poll();
    return () => clearInterval(interval);
  }, [isEnabled]);

  return entries;
}
