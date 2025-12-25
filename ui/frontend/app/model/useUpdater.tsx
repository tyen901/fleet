import { useEffect, useRef, useState } from "react";
import {
  subscribeUpdateState,
  updateApply,
  updateCheck,
  type UpdateModel,
} from "../../api";

export function useUpdater() {
  const [model, setModel] = useState<UpdateModel | null>(null);
  const latestRef = useRef<UpdateModel | null>(null);

  useEffect(() => {
    let raf: number;

    const loop = () => {
      if (latestRef.current) {
        setModel(latestRef.current);
      }
      raf = requestAnimationFrame(loop);
    };

    raf = requestAnimationFrame(loop);

    const subscription = subscribeUpdateState((next) => {
      latestRef.current = next;
    });

    return () => {
      cancelAnimationFrame(raf);
      void subscription.then((dispose) => dispose());
    };
  }, []);

  return {
    model,
    check: updateCheck,
    apply: updateApply,
  };
}
