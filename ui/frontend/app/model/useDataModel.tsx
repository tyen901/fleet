import { useCallback, useEffect, useState } from "react";
import {
  dataSnapshot,
  dataSelectProfile,
  dataRefreshProfiles,
  dataCreateProfile,
  dataUpdateProfile,
  dataDeleteProfile,
  dataSetSettings,
  dataResetSettingsToDefaults,
  dataRequestLaunchArgsPreview,
  dataRequestRepoSpec,
  dataRequestLinuxValidation,
  dataRequestLinuxValidationWithSettings,
  type DataModel,
  type ProfileCreate,
  type ProfileUpdate,
  type LaunchSettings,
} from "../../api";

const describeError = (value: unknown) =>
  value instanceof Error ? value.message : String(value);

export function useDataModel() {
  const [data, setData] = useState<DataModel | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const snap = await dataSnapshot();
      setData(snap);
      setError(null);
    } catch (e: unknown) {
      setError(describeError(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const refreshProfiles = useCallback(async () => {
    await dataRefreshProfiles();
    await refresh();
  }, [refresh]);

  const selectProfile = useCallback(
    async (id: string) => {
      await dataSelectProfile(id);
      await refresh();
    },
    [refresh],
  );

  const createProfile = useCallback(
    async (create: ProfileCreate) => {
      await dataCreateProfile(create);
      await refresh();
    },
    [refresh],
  );

  const updateProfile = useCallback(
    async (id: string, update: ProfileUpdate) => {
      await dataUpdateProfile(id, update);
      await refresh();
    },
    [refresh],
  );

  const deleteProfile = useCallback(
    async (id: string) => {
      await dataDeleteProfile(id);
      await refresh();
    },
    [refresh],
  );

  const saveSettings = useCallback(
    async (settings: LaunchSettings) => {
      await dataSetSettings(settings);
      await refresh();
    },
    [refresh],
  );

  const resetSettings = useCallback(async () => {
    await dataResetSettingsToDefaults();
    await refresh();
  }, [refresh]);

  const requestLaunchPreview = useCallback(
    async (profileId: string) => {
      await dataRequestLaunchArgsPreview(profileId);
      await refresh();
    },
    [refresh],
  );

  const requestRepoSpec = useCallback(
    async (profileId: string) => {
      await dataRequestRepoSpec(profileId);
      // repo spec fetch is async in service; do a few refreshes to pick it up.
      for (let i = 0; i < 12; i++) {
        await new Promise((r) => setTimeout(r, 300));
        await refresh();
      }
    },
    [refresh],
  );

  const requestLinuxValidation = useCallback(
    async (profileId: string) => {
      await dataRequestLinuxValidation(profileId);
      await refresh();
    },
    [refresh],
  );

  const requestLinuxValidationWithSettings = useCallback(
    async (profileId: string, settings: LaunchSettings) => {
      await dataRequestLinuxValidationWithSettings(profileId, settings);
      await refresh();
    },
    [refresh],
  );

  return {
    data,
    error,
    refresh,
    refreshProfiles,
    selectProfile,
    createProfile,
    updateProfile,
    deleteProfile,
    saveSettings,
    resetSettings,
    requestLaunchPreview,
    requestRepoSpec,
    requestLinuxValidation,
    requestLinuxValidationWithSettings,
  };
}
