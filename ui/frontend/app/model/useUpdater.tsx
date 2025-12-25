import { useEffect, useRef, useState } from "react";
import {
  subscribeUpdateState,
  updateCheck,
  updateApply,
  type UpdateModel,
} from "../../api";

export function useUpdater() {
  const [state, setState] = useState<UpdateModel | null>(null);
  const latestRef = useRef<UpdateModel | null>(null);

  useEffect(() => {
    let raf: number;
    const loop = () => {
      if (latestRef.current) setState(latestRef.current);
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);

    const sub = subscribeUpdateState((next) => {
      latestRef.current = next;
    });

    return () => {
      cancelAnimationFrame(raf);
      void sub.then((dispose) => dispose?.());
    };
  }, []);

  return { state, check: updateCheck, apply: updateApply };
}
