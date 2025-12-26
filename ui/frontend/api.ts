import { invoke, Channel } from "@tauri-apps/api/core";

// -------------------- Error handling --------------------

export type ApiError = { code: string; message: string };

function normalizeInvokeError(e: unknown): ApiError {
  // Tauri typically throws something string-like or object-like.
  if (typeof e === "string") return { code: "invoke_error", message: e };
  if (e && typeof e === "object") {
    const errObj = e as { message?: unknown; code?: unknown };
    if (typeof errObj.message === "string") {
      const code =
        typeof errObj.code === "string" ? errObj.code : "invoke_error";
      return { code, message: errObj.message };
    }
    try {
      return { code: "invoke_error", message: JSON.stringify(e) };
    } catch {
      return { code: "invoke_error", message: String(e) };
    }
  }
  return { code: "invoke_error", message: String(e) };
}

async function invokeSafe<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw normalizeInvokeError(e);
  }
}

// -------------------- Domain Types --------------------

// Note: ProfileSpec is NOT camelCase in Rust (no rename_all on ProfileSpec)
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

// LaunchSettings (nested inside DataModel) is snake_case in Rust.
export type OpenMode = "system_default" | "linux_flatpak_host";

export type WindowsLaunchMethod =
  | "direct_exe"
  | "steam_app_launch"
  | "steam_uri";

export type LinuxModPathStyle = "native" | "proton_z";

export type LaunchSettings = {
  open_mode: OpenMode;
  arma3: {
    windows: {
      method: WindowsLaunchMethod;
      arma3_exe: string | null;
      steam_exe: string | null;
    };
    linux: {
      template: string;
      mod_path_style: LinuxModPathStyle;
      shell: string | null;
    };
  };
};

export type SyncMode = "repair" | "sync_fresh" | "check" | "verify";
export type SafeWipePolicy =
  | "none"
  | "expected_from_store_baseline"
  | "expected_from_remote_manifest"
  | "expected_union";
export type UnknownPathPolicy = "keep" | "delete";
export type UnexpectedPathPolicy = "prompt" | "delete";

export type SyncTuning = {
  mode: SyncMode;

  full_download_part_threshold: number;
  full_download_byte_ratio_threshold: number;

  patch_max_fetch_ratio: number;
  patch_merge_gap_bytes: number;
  patch_min_range_bytes: number;
  patch_max_range_requests: number | null;

  max_concurrent_files: number | null;
  max_concurrent_range_requests: number | null;
  scan_concurrency: number;

  io_buffer_bytes: number;
  use_index: boolean;
  emit_progress: boolean;
  auto_fix_case: boolean;

  unexpected_paths: UnexpectedPathPolicy;
  max_unexpected_delete_bytes: number | null;
  delete_empty_dirs: boolean;

  safe_wipe: SafeWipePolicy;
  unknown_paths: UnknownPathPolicy;

  enable_patch_repair: boolean;
  enable_skip_check: boolean;
};

export function defaultSyncTuning(mode: SyncMode): SyncTuning {
  // Must match crates/fleet_app/src/sync/mod.rs Default for SyncTuning
  return {
    mode,

    full_download_part_threshold: 256,
    full_download_byte_ratio_threshold: 0.6,

    patch_max_fetch_ratio: 2.0, // from RepairTuning default (mirrors Rust default; keep stable)
    patch_merge_gap_bytes: 64 * 1024,
    patch_min_range_bytes: 128 * 1024,
    patch_max_range_requests: null,

    max_concurrent_files: null,
    max_concurrent_range_requests: null,
    scan_concurrency: 8,

    io_buffer_bytes: 1024 * 1024,
    use_index: true,
    emit_progress: true,
    auto_fix_case: true,

    unexpected_paths: "prompt",
    max_unexpected_delete_bytes: null,
    delete_empty_dirs: true,

    safe_wipe: "expected_union",
    unknown_paths: "delete",

    enable_patch_repair: true,
    enable_skip_check: true,
  };
}

// SyncReadModel IS camelCase in Rust.
export type SyncReadModel = {
  phase: string;
  percent: number;
  bytesDone: number;
  bytesTotal: number;
  filesVerified: number;
  filesUpToDate: number;
  throughputBps: number;
  etaSeconds: number | null;
  error: string | null;
  finished: boolean;
  statusLine: string;
  canStart: boolean;
  canCancel: boolean;
};

export type LogEntry = {
  seq: number;
  message: string;
  level: string;
};

// LogPage is snake_case for next_cursor in Rust.
export type LogPage = {
  entries: LogEntry[];
  next_cursor: number;
};

// Update model types (Rust uses camelCase tagging for UpdateState)
export type UpdateState =
  | { type: "notConfigured" }
  | { type: "idle"; status: string }
  | { type: "checking" }
  | { type: "downloading"; progress: number | null }
  | { type: "failed"; error: string };

export type UpdateModel = {
  state: UpdateState;
  available: unknown | null; // velopack UpdateInfo has non-idiomatic casing; treat as unknown for UI display
};

// Extra cached fields returned by Rust DataModel (camelCase).
export type LinuxTemplateValidation = {
  ok: boolean;
  errors: string[];
  warnings: string[];
  normalized_template: string;
  preview: string;
};

export type DataModel = {
  warning: string | null;

  profiles: ProfileSpec[];
  selectedId: string | null;

  settings: LaunchSettings;

  launchArgsPreview: string | null;
  launchArgsError: string | null;

  repoSpec: unknown | null;
  repoSpecError: string | null;
  repoSpecGeneration: number;

  linuxValidation: LinuxTemplateValidation | null;
  linuxValidationError: string | null;

  lastSyncOutcome: unknown | null;
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

// -------------------- Commands --------------------

// Data
export function dataSnapshot(): Promise<DataModel> {
  return invokeSafe("data_snapshot");
}
export function dataRefreshProfiles(): Promise<void> {
  return invokeSafe("data_refresh_profiles");
}
export function dataSelectProfile(id: string): Promise<void> {
  return invokeSafe("data_select_profile", { id });
}
export function dataCreateProfile(create: ProfileCreate): Promise<string> {
  return invokeSafe("data_create_profile", { create });
}
export function dataUpdateProfile(
  id: string,
  update: ProfileUpdate,
): Promise<void> {
  return invokeSafe("data_update_profile", { id, update });
}
export function dataDeleteProfile(id: string): Promise<void> {
  return invokeSafe("data_delete_profile", { id });
}
export function dataLaunchArma3(id: string): Promise<void> {
  return invokeSafe("data_launch_arma3", { id });
}
export function dataOpenCheckoutRoot(profile_id: string): Promise<void> {
  return invokeSafe("data_open_checkout_root", { profile_id });
}
export function dataOpenFolder(path: string): Promise<void> {
  return invokeSafe("data_open_folder", { path });
}
export function dataSetSettings(settings: LaunchSettings): Promise<void> {
  return invokeSafe("data_set_settings", { settings });
}
export function dataResetSettingsToDefaults(): Promise<void> {
  return invokeSafe("data_reset_settings_to_defaults");
}
export function dataLaunchArgsPreview(profile_id: string): Promise<string> {
  return invokeSafe("data_launch_args_preview", { profile_id });
}
export function dataRequestLaunchArgsPreview(
  profile_id: string,
): Promise<void> {
  return invokeSafe("data_request_launch_args_preview", { profile_id });
}
export function dataRequestRepoSpec(profile_id: string): Promise<void> {
  return invokeSafe("data_request_repo_spec", { profile_id });
}
export function dataRequestRepoSpecForUrl(repo_url: string): Promise<void> {
  return invokeSafe("data_request_repo_spec_for_url", { repo_url });
}
export function dataRequestLinuxValidation(profile_id: string): Promise<void> {
  return invokeSafe("data_request_linux_validation", { profile_id });
}
export function dataRequestLinuxValidationWithSettings(
  profile_id: string,
  settings: LaunchSettings,
): Promise<void> {
  return invokeSafe("data_request_linux_validation_with_settings", {
    profile_id,
    settings,
  });
}
export function dataRebuildIndex(profile_id: string): Promise<void> {
  return invokeSafe("data_rebuild_index", { profile_id });
}
export function dataClearCache(profile_id: string): Promise<void> {
  return invokeSafe("data_clear_cache", { profile_id });
}
export function dataClearLastSyncOutcome(): Promise<void> {
  return invokeSafe("data_clear_last_sync_outcome");
}
export function dataInitRegistry(): Promise<void> {
  return invokeSafe("data_init_registry");
}
export function dataRegistryPath(): Promise<string> {
  return invokeSafe("data_registry_path");
}

// Sync
export function syncSnapshot(): Promise<SyncReadModel> {
  return invokeSafe("sync_snapshot");
}
export function syncStart(mode: SyncMode, tuning: SyncTuning): Promise<void> {
  return invokeSafe("sync_start", { mode, tuning });
}
export function syncCancel(): Promise<void> {
  return invokeSafe("sync_cancel");
}
export async function subscribeSyncState(
  onMessage: (s: SyncReadModel) => void,
): Promise<() => void> {
  const ch = new Channel<SyncReadModel>();
  ch.onmessage = onMessage;
  await invokeSafe("subscribe_sync_state", { onSnapshot: ch });
  return () => {
    // Tauri Channel dispose is not currently exposed in JS; keep single subscription in app lifetime.
  };
}
export function getSyncLogs(cursor: number): Promise<LogPage> {
  return invokeSafe("get_sync_logs", { cursor });
}

// Update
export function updateSnapshot(): Promise<UpdateModel> {
  return invokeSafe("update_snapshot");
}
export function updateCheck(): Promise<void> {
  return invokeSafe("update_check");
}
export function updateApply(): Promise<void> {
  return invokeSafe("update_apply");
}
export function updateClearError(): Promise<void> {
  return invokeSafe("update_clear_error");
}
export async function subscribeUpdateState(
  onState: (s: UpdateModel) => void,
): Promise<() => void> {
  const ch = new Channel<UpdateModel>();
  ch.onmessage = onState;
  await invokeSafe("subscribe_update_state", { onState: ch });
  return () => {
    // Same note as sync subscription.
  };
}
