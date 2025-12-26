import { useEffect, useState } from "react";
import {
  subscribeSyncState,
  syncSnapshot,
  type SyncReadModel,
} from "../../api";

export function useSyncJob() {
  const [snapshot, setSnapshot] = useState<SyncReadModel | null>(null);

  useEffect(() => {
    let dispose: (() => void) | null = null;

    void (async () => {
      try {
        const initial = await syncSnapshot();
        setSnapshot(initial);
      } catch {
        // ignore; subscription may still deliver
      }

      const d = await subscribeSyncState((next) => {
        setSnapshot(next);
      });
      dispose = d;
    })();

    return () => {
      dispose?.();
    };
  }, []);

  return snapshot;
}
