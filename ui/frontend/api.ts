import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

// --- Types from backend ---

export interface ProfileSpec {
  id: string;
  name: string;
  repo_url: string;
  checkout_root: string;
  last_sync_unix_s: number | null;
  arma3_extra_args: string;
  arma3_enabled_mods: string[];
}

export interface DataModel {
  warning: string | null;
  profiles: ProfileSpec[];
  selected_id: string | null;
  // Incomplete for brevity
}

export interface SyncReadModel {
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
}

export type UpdateState =
  | { type: "notConfigured" }
  | { type: "idle"; status: string }
  | { type: "checking" }
  | { type: "downloading"; progress: number | null }
  | { type: "failed"; error: string };

export interface UpdateModel {
  state: UpdateState;
  available: unknown | null; // UpdateInfo is complex, using unknown for now
}

// --- Commands ---

export function dataSnapshot(): Promise<DataModel> {
  return invoke("get_data_snapshot");
}

export function syncStart(mode: "repair" | "fresh" | "check"): Promise<void> {
  return invoke("sync_start", { mode });
}

export function syncCancel(): Promise<void> {
  return invoke("sync_cancel");
}

export function updateCheck(): Promise<void> {
  return invoke("update_check");
}

// --- Subscriptions ---

export async function subscribeSyncState(
  callback: (model: SyncReadModel) => void
): Promise<() => void> {
  await invoke("subscribe_sync_state");
  return await listen<SyncReadModel>("sync-state", (event) => {
    callback(event.payload);
  });
}

export async function subscribeUpdateState(
  callback: (model: UpdateModel) => void
): Promise<() => void> {
  await invoke("subscribe_update_state");
  return await listen<UpdateModel>("update-state", (event) => {
    callback(event.payload);
  });
}
