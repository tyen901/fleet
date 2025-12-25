import { useEffect, useRef, useState } from "react";
import { getSyncLogs, type LogEntry } from "../../api";

export function useSyncLog(isEnabled: boolean) {
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const cursorRef = useRef(0);

  useEffect(() => {
    if (!isEnabled) {
      cursorRef.current = 0;
      setEntries([]);
      return;
    }

    const poll = async () => {
      try {
        const page = await getSyncLogs(cursorRef.current);
        if (page.entries.length > 0) {
          cursorRef.current = page.next_cursor;
          setEntries((prev) => [...prev, ...page.entries].slice(-1000));
        }
      } catch (err) {
        console.error(err);
      }
    };

    const id = window.setInterval(poll, 500);
    void poll();
    return () => window.clearInterval(id);
  }, [isEnabled]);

  return entries;
}
