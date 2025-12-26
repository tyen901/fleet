import { useEffect, useState } from "react";
import {
  subscribeUpdateState,
  updateSnapshot,
  updateCheck,
  updateApply,
  updateClearError,
  type UpdateModel,
} from "../../api";

export function useUpdater() {
  const [state, setState] = useState<UpdateModel | null>(null);

  useEffect(() => {
    let dispose: (() => void) | null = null;

    void (async () => {
      try {
        const initial = await updateSnapshot();
        setState(initial);
      } catch {
        // ignore
      }

      const d = await subscribeUpdateState((next) => setState(next));
      dispose = d;
    })();

    return () => {
      dispose?.();
    };
  }, []);

  return {
    state,
    check: updateCheck,
    apply: updateApply,
    clearError: updateClearError,
  };
}
