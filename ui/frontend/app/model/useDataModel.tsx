import { useCallback, useEffect, useState } from "react";
import { dataSnapshot, dataSelectProfile, type DataModel } from "../../api";

export function useDataModel() {
  const [data, setData] = useState<DataModel | null>(null);

  const refresh = useCallback(async () => {
    const snap = await dataSnapshot();
    setData(snap);
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selectProfile = useCallback(
    async (id: string) => {
      await dataSelectProfile(id);
      await refresh();
    },
    [refresh]
  );

  return { data, selectProfile, refresh };
}
