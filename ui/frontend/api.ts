import { invoke, Channel } from "@tauri-apps/api/core";

// -------------------- Types --------------------

export type ApiError = {
  code: string;
  message: string;
};

export type Arma3Config = {
  extra_args: string;
  enabled_mods: string[];
};

export type ProfileSpec = {
  id: string;
  name: string;
  repo_url: string;
  checkout_root: string;
  created_unix_s: number;
  last_sync_unix_s: number | null;
  arma3: Arma3Config;
};

export type AppSettings = {
  open_mode: "system_default" | "linux_flatpak_host";
};

export type DataModel = {
  warning: string | null;
  profiles: ProfileSpec[];
  selected_id: string | null;
  settings: AppSettings;
  launch_args_preview: string | null;
  launch_args_error: string | null;
};

export type ProfileCreate = {
  name: string;
  repo_url: string;
  checkout_root: string;
  select: boolean;
  arma3_extra_args: string;
  arma3_enabled_mods: string[];
};

export type ProfileUpdate = {
  name?: string | null;
  repo_url?: string | null;
  checkout_root?: string | null;
  select?: boolean | null;
  arma3_extra_args?: string | null;
  arma3_enabled_mods?: string[] | null;
};

export type SyncMode = "repair" | "sync_fresh" | "check" | "verify";

export type SyncTuning = {
  mode: SyncMode;
  full_download_part_threshold: number;
  full_download_byte_ratio_threshold: number;
  max_concurrent_files?: number;
  max_concurrent_range_requests?: number;
  io_buffer_bytes: number;
  use_index: boolean;
  emit_progress: boolean;
  delete_unexpected?: boolean;
};

export type SyncReadModel = {
  phase: string;
  percent: number;
  bytes_done: number;
  bytes_total: number;
  files_verified: number;
  files_up_to_date: number;
  error: string | null;
  finished: boolean;
  can_start: boolean;
  can_cancel: boolean;
  status_line: string;
};

export type UpdateState =
  | { type: "notConfigured" }
  | { type: "idle"; status: string }
  | { type: "checking" }
  | { type: "downloading"; progress: number | null }
  | { type: "failed"; error: string };

export type UpdateInfo = {
  TargetFullRelease: {
    Version: string;
  };
  IsDowngrade: boolean;
};

export type UpdateModel = {
  state: UpdateState;
  available: UpdateInfo | null;
};

// -------------------- Commands --------------------

export async function dataSnapshot(): Promise<DataModel> {
  return invoke("data_snapshot");
}

export async function dataRefreshProfiles(): Promise<void> {
  return invoke("data_refresh_profiles");
}

export async function dataSelectProfile(id: string): Promise<void> {
  return invoke("data_select_profile", { id });
}

export async function dataCreateProfile(create: ProfileCreate): Promise<string> {
  return invoke("data_create_profile", { create });
}

export async function dataUpdateProfile(id: string, update: ProfileUpdate): Promise<void> {
  return invoke("data_update_profile", { id, update });
}

export async function dataDeleteProfile(id: string): Promise<void> {
  return invoke("data_delete_profile", { id });
}

export async function dataLaunchArma3(id: string): Promise<void> {
  return invoke("data_launch_arma3", { id });
}

export async function syncSnapshot(): Promise<SyncReadModel> {
  return invoke("sync_snapshot");
}

export async function syncStart(mode: SyncMode, tuning?: Partial<SyncTuning>): Promise<void> {
  const defaultTuning: SyncTuning = {
    mode,
    full_download_part_threshold: 256,
    full_download_byte_ratio_threshold: 0.6,
    io_buffer_bytes: 1024 * 1024,
    use_index: true,
    emit_progress: true,
  };

  return invoke("sync_start", { mode, tuning: { ...defaultTuning, ...tuning } });
}

export async function syncCancel(): Promise<void> {
  return invoke("sync_cancel");
}

export async function subscribeSyncState(onState: (s: SyncReadModel) => void): Promise<void> {
  const ch = new Channel<SyncReadModel>();
  ch.onmessage = onState;
  return invoke("subscribe_sync_state", { onState: ch });
}

export async function updateSnapshot(): Promise<UpdateModel> {
  return invoke("update_snapshot");
}

export async function updateCheck(): Promise<void> {
  return invoke("update_check");
}

export async function updateApply(): Promise<void> {
  return invoke("update_apply");
}

export async function subscribeUpdateState(onState: (s: UpdateModel) => void): Promise<void> {
  const ch = new Channel<UpdateModel>();
  ch.onmessage = onState;
  return invoke("subscribe_update_state", { onState: ch });
}
