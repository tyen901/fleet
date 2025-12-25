import { useState, useEffect, useRef } from "react";
import { subscribeSyncState, type SyncReadModel } from "../../api";

export function useSyncJob() {
  const [snapshot, setSnapshot] = useState<SyncReadModel | null>(null);
  const latestRef = useRef<SyncReadModel | null>(null);

  useEffect(() => {
    let rafId: number;

    const loop = () => {
      if (latestRef.current) {
        setSnapshot(latestRef.current);
      }
      rafId = requestAnimationFrame(loop);
    };

    rafId = requestAnimationFrame(loop);

    const subscription = subscribeSyncState((next) => {
      latestRef.current = next;
    });

    return () => {
      cancelAnimationFrame(rafId);
      void subscription.then((dispose) => dispose());
    };
  }, []);

  return snapshot;
}
