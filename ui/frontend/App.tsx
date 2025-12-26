import { useMemo, useState } from "react";
import {
  dataLaunchArma3,
  dataOpenCheckoutRoot,
  dataRegistryPath,
  dataInitRegistry,
  dataRebuildIndex,
  dataClearCache,
  syncStart,
  syncCancel,
  defaultSyncTuning,
  type ProfileCreate,
  type ProfileUpdate,
  type LaunchSettings,
  type SyncMode,
  type OpenMode,
  type WindowsLaunchMethod,
  type LinuxModPathStyle,
} from "./api";

import { useDataModel } from "./app/model/useDataModel";
import { useSyncJob } from "./app/model/useSyncJob";
import { useSyncLog } from "./app/model/useSyncLog";
import { useUpdater } from "./app/model/useUpdater";

import "./App.css";

type ProfileDraft = {
  id?: string;
  name: string;
  repo_url: string;
  checkout_root: string;
  select: boolean;
  arma3_extra_args: string;
  arma3_enabled_mods_csv: string;
};

function modsCsvToList(csv: string): string[] {
  return csv
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean);
}

function modsListToCsv(list: string[]): string {
  return list.join(", ");
}

const describeError = (value: unknown) =>
  value instanceof Error ? value.message : String(value);

function cloneSettings(base: LaunchSettings): LaunchSettings {
  return structuredClone(base) as LaunchSettings;
}

export default function App() {
  const dm = useDataModel();
  const sync = useSyncJob();
  const logs = useSyncLog(Boolean(sync && !sync.finished));
  const updater = useUpdater();

  const data = dm.data;
  const selectedId = data?.selectedId ?? null;

  const activeProfile = useMemo(() => {
    if (!data || !selectedId) return null;
    return data.profiles.find((p) => p.id === selectedId) ?? null;
  }, [data, selectedId]);

  const [uiError, setUiError] = useState<string | null>(null);

  const [isEditingProfile, setIsEditingProfile] = useState(false);
  const [profileDraft, setProfileDraft] = useState<ProfileDraft | null>(null);

  const [settingsDraft, setSettingsDraft] = useState<LaunchSettings | null>(
    null,
  );

  const [deleteUnexpected, setDeleteUnexpected] = useState(false);

  const openCreateProfile = () => {
    setUiError(null);
    setProfileDraft({
      name: "",
      repo_url: "",
      checkout_root: "",
      select: true,
      arma3_extra_args: "",
      arma3_enabled_mods_csv: "",
    });
    setIsEditingProfile(false);
  };

  const openEditProfile = () => {
    if (!activeProfile) return;
    setUiError(null);
    setProfileDraft({
      id: activeProfile.id,
      name: activeProfile.name,
      repo_url: activeProfile.repo_url,
      checkout_root: activeProfile.checkout_root,
      select: true,
      arma3_extra_args: activeProfile.arma3?.extra_args ?? "",
      arma3_enabled_mods_csv: modsListToCsv(
        activeProfile.arma3?.enabled_mods ?? [],
      ),
    });
    setIsEditingProfile(true);
  };

  const closeProfileModal = () => setProfileDraft(null);

  const submitProfile = async () => {
    if (!profileDraft) return;
    setUiError(null);

    try {
      if (isEditingProfile && profileDraft.id) {
        const update: ProfileUpdate = {
          name: profileDraft.name,
          repo_url: profileDraft.repo_url,
          checkout_root: profileDraft.checkout_root,
          select: profileDraft.select,
          arma3_extra_args: profileDraft.arma3_extra_args,
          arma3_enabled_mods: modsCsvToList(
            profileDraft.arma3_enabled_mods_csv,
          ),
        };
        await dm.updateProfile(profileDraft.id, update);
      } else {
        const create: ProfileCreate = {
          name: profileDraft.name,
          repo_url: profileDraft.repo_url,
          checkout_root: profileDraft.checkout_root,
          select: profileDraft.select,
          arma3_extra_args: profileDraft.arma3_extra_args,
          arma3_enabled_mods: modsCsvToList(
            profileDraft.arma3_enabled_mods_csv,
          ),
        };
        await dm.createProfile(create);
      }
      closeProfileModal();
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const deleteSelectedProfile = async () => {
    if (!activeProfile) return;
    setUiError(null);
    if (!confirm(`Delete profile "${activeProfile.name}"?`)) return;
    try {
      await dm.deleteProfile(activeProfile.id);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const doLaunch = async () => {
    if (!activeProfile) return;
    setUiError(null);
    try {
      await dataLaunchArma3(activeProfile.id);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const doOpenRoot = async () => {
    if (!activeProfile) return;
    setUiError(null);
    try {
      await dataOpenCheckoutRoot(activeProfile.id);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const doStartSync = async (mode: SyncMode) => {
    setUiError(null);
    try {
      const tuning = defaultSyncTuning(mode);
      tuning.unexpected_paths = deleteUnexpected ? "delete" : "prompt";
      await syncStart(mode, tuning);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const doCancelSync = async () => {
    setUiError(null);
    try {
      await syncCancel();
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const ensureSettingsDraft = () => {
    if (!data) return;
    if (!settingsDraft) setSettingsDraft(cloneSettings(data.settings));
  };

  const saveSettings = async () => {
    if (!activeProfile || !settingsDraft) return;
    setUiError(null);
    try {
      await dm.saveSettings(settingsDraft);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const resetSettings = async () => {
    setUiError(null);
    try {
      await dm.resetSettings();
      setSettingsDraft(null);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const validateLinuxTemplate = async () => {
    if (!activeProfile || !settingsDraft) return;
    setUiError(null);
    try {
      await dm.requestLinuxValidationWithSettings(
        activeProfile.id,
        settingsDraft,
      );
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const fetchRepoSpec = async () => {
    if (!activeProfile) return;
    setUiError(null);
    try {
      await dm.requestRepoSpec(activeProfile.id);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const refreshLaunchPreview = async () => {
    if (!activeProfile) return;
    setUiError(null);
    try {
      await dm.requestLaunchPreview(activeProfile.id);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const getRegistryPath = async () => {
    setUiError(null);
    try {
      const p = await dataRegistryPath();
      alert(p);
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const initRegistry = async () => {
    setUiError(null);
    try {
      await dataInitRegistry();
      await dm.refresh();
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const rebuildIndex = async () => {
    if (!activeProfile) return;
    setUiError(null);
    try {
      await dataRebuildIndex(activeProfile.id);
      alert("Index cleared (will rebuild on next sync).");
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const clearCache = async () => {
    if (!activeProfile) return;
    setUiError(null);
    try {
      await dataClearCache(activeProfile.id);
      alert("Cache cleared.");
    } catch (e: unknown) {
      setUiError(describeError(e));
    }
  };

  const updateModel = updater.state;
  const updateState = updateModel?.state ?? null;
  const updateAvailable = updateModel?.available ?? null;

  return (
    <div className="shell">
      <header className="app-header">
        <h1>FLEET</h1>
        <div className="row">
          <span className="badge">GUI</span>
          <span className="version-tag">v0.1.0</span>
        </div>
      </header>

      <div className="layout">
        <aside className="sidebar">
          <h3>Profiles</h3>

          <div className="profile-list">
            {data?.profiles.map((p) => (
              <button
                key={p.id}
                className={`profile-btn ${p.id === selectedId ? "active" : ""}`}
                onClick={() => dm.selectProfile(p.id)}
              >
                {p.name}
                <div className="muted">{p.repo_url}</div>
              </button>
            ))}
            {!data && <div className="muted">Loading…</div>}
            {data && data.profiles.length === 0 && (
              <div className="muted">No profiles. Create one to begin.</div>
            )}
          </div>

          <div className="sidebar-actions">
            <button onClick={openCreateProfile}>Add</button>
            <button onClick={openEditProfile} disabled={!activeProfile}>
              Edit
            </button>
            <button
              className="danger"
              onClick={deleteSelectedProfile}
              disabled={!activeProfile}
            >
              Delete
            </button>
          </div>

          <div className="sidebar-actions">
            <button onClick={() => dm.refreshProfiles()}>Refresh</button>
            <button onClick={initRegistry}>Init registry</button>
          </div>

          <div className="sidebar-actions">
            <button onClick={getRegistryPath}>Registry path</button>
          </div>
        </aside>

        <main className="content">
          {data?.warning && (
            <div className="card">
              <h3>Warning</h3>
              <div className="error">{data.warning}</div>
            </div>
          )}

          {(dm.error || uiError) && (
            <div className="card">
              <h3>Error</h3>
              <div className="error">{uiError ?? dm.error}</div>
            </div>
          )}

          {/* Selected profile */}
          <section className="card">
            <div className="row space">
              <h2>Profile</h2>
              <div className="row">
                <button onClick={doOpenRoot} disabled={!activeProfile}>
                  Open folder
                </button>
                <button
                  className="primary"
                  onClick={doLaunch}
                  disabled={!activeProfile || Boolean(sync && !sync.finished)}
                >
                  Launch Arma 3
                </button>
              </div>
            </div>

            {activeProfile ? (
              <>
                <div className="kv-grid">
                  <span className="label">Name</span>
                  <span className="value">{activeProfile.name}</span>
                  <span className="label">Repository</span>
                  <span className="value">{activeProfile.repo_url}</span>
                  <span className="label">Path</span>
                  <span className="value">{activeProfile.checkout_root}</span>
                  <span className="label">Last sync</span>
                  <span className="value">
                    {activeProfile.last_sync_unix_s
                      ? String(activeProfile.last_sync_unix_s)
                      : "—"}
                  </span>
                </div>

                <div className="hr" />

                <div className="row">
                  <button onClick={refreshLaunchPreview}>
                    Compute launch preview
                  </button>
                  <button onClick={fetchRepoSpec}>Fetch repo spec</button>
                  <button onClick={rebuildIndex}>Clear index</button>
                  <button onClick={clearCache}>Clear cache</button>
                </div>

                <div className="hr" />

                <div className="row">
                  <div className="muted">Preview:</div>
                </div>
                {data?.launchArgsError && (
                  <div className="error">{data.launchArgsError}</div>
                )}
                {data?.launchArgsPreview && (
                  <pre className="log-viewer" style={{ maxHeight: 140 }}>
                    {data.launchArgsPreview}
                  </pre>
                )}

                <div className="row">
                  <div className="muted">Repo spec:</div>
                </div>
                {data?.repoSpecError && (
                  <div className="error">{data.repoSpecError}</div>
                )}
                {data?.repoSpec && (
                  <pre className="log-viewer" style={{ maxHeight: 180 }}>
                    {JSON.stringify(data.repoSpec, null, 2)}
                  </pre>
                )}
              </>
            ) : (
              <div className="muted">Select a profile to begin.</div>
            )}
          </section>

          {/* Sync */}
          <section className="card">
            <div className="row space">
              <h2>Sync</h2>
              <div className="row">
                <label className="row" style={{ gap: 8 }}>
                  <input
                    type="checkbox"
                    checked={deleteUnexpected}
                    onChange={(e) => setDeleteUnexpected(e.target.checked)}
                  />
                  <span className="small">Delete unexpected (no prompt)</span>
                </label>
                <button
                  className="danger"
                  onClick={doCancelSync}
                  disabled={!sync?.canCancel}
                >
                  Cancel
                </button>
              </div>
            </div>

            <div className="row" style={{ marginTop: 8 }}>
              <button
                onClick={() => doStartSync("repair")}
                disabled={!activeProfile || !sync?.canStart}
              >
                Repair
              </button>
              <button
                onClick={() => doStartSync("sync_fresh")}
                disabled={!activeProfile || !sync?.canStart}
              >
                Fresh
              </button>
              <button
                onClick={() => doStartSync("check")}
                disabled={!activeProfile || !sync?.canStart}
              >
                Check
              </button>
              <button
                onClick={() => doStartSync("verify")}
                disabled={!activeProfile || !sync?.canStart}
              >
                Verify
              </button>
            </div>

            <div className="hr" />

            <div className="row space">
              <div>
                <div className="muted">Status</div>
                <div>{sync?.statusLine ?? "—"}</div>
              </div>
              <div className="row">
                <span className="badge">
                  {sync?.throughputBps ? `${sync.throughputBps} B/s` : "—"}
                </span>
                <span className="badge">
                  {sync?.etaSeconds != null
                    ? `ETA ${sync.etaSeconds}s`
                    : "ETA —"}
                </span>
              </div>
            </div>

            <div className="progress">
              <div className="progress-track">
                <div
                  className="progress-fill"
                  style={{ width: `${sync?.percent ?? 0}%` }}
                />
              </div>
              <div className="pct">{sync?.percent ?? 0}%</div>
            </div>

            <div className="row" style={{ marginTop: 8 }}>
              <span className="badge">Verified {sync?.filesVerified ?? 0}</span>
              <span className="badge">
                Up-to-date {sync?.filesUpToDate ?? 0}
              </span>
              <span className="badge">
                Bytes {sync?.bytesDone ?? 0}/{sync?.bytesTotal ?? 0}
              </span>
            </div>

            {sync?.error && (
              <div className="error" style={{ marginTop: 8 }}>
                {sync.error}
              </div>
            )}

            <div className="log-viewer">
              {logs.length > 0 ? (
                logs.map((entry) => (
                  <div
                    key={entry.seq}
                    className={`log-line ${entry.level.toLowerCase()}`}
                  >
                    [{entry.seq}] {entry.level.toUpperCase()}: {entry.message}
                  </div>
                ))
              ) : (
                <div className="muted">No recent logs.</div>
              )}
            </div>
          </section>

          {/* Settings */}
          <section className="card" onMouseEnter={ensureSettingsDraft}>
            <div className="row space">
              <h2>Settings</h2>
              <div className="row">
                <button onClick={resetSettings}>Reset defaults</button>
                <button
                  className="primary"
                  onClick={saveSettings}
                  disabled={!settingsDraft}
                >
                  Save
                </button>
              </div>
            </div>

            {!data ? (
              <div className="muted">Loading…</div>
            ) : (
              <>
                <div className="form-grid">
                  <div className="label">Open mode</div>
                  <div>
                    <select
                      value={(settingsDraft ?? data.settings).open_mode}
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.open_mode = e.target.value as OpenMode;
                        setSettingsDraft(next);
                      }}
                    >
                      <option value="system_default">system_default</option>
                      <option value="linux_flatpak_host">
                        linux_flatpak_host
                      </option>
                    </select>
                    <div className="small">
                      Controls folder/URL opening behavior.
                    </div>
                  </div>

                  <div className="label">Linux template</div>
                  <div>
                    <input
                      value={
                        (settingsDraft ?? data.settings).arma3.linux.template
                      }
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.arma3.linux.template = e.target.value;
                        setSettingsDraft(next);
                      }}
                    />
                    <div className="small">
                      Must include $ARGS and/or $MODS.
                    </div>
                  </div>

                  <div className="label">Linux mod paths</div>
                  <div>
                    <select
                      value={
                        (settingsDraft ?? data.settings).arma3.linux
                          .mod_path_style
                      }
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.arma3.linux.mod_path_style = e.target
                          .value as LinuxModPathStyle;
                        setSettingsDraft(next);
                      }}
                    >
                      <option value="native">native</option>
                      <option value="proton_z">proton_z</option>
                    </select>
                  </div>

                  <div className="label">Linux shell</div>
                  <div>
                    <input
                      placeholder="(optional)"
                      value={
                        (settingsDraft ?? data.settings).arma3.linux.shell ?? ""
                      }
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.arma3.linux.shell = e.target.value.trim()
                          ? e.target.value
                          : null;
                        setSettingsDraft(next);
                      }}
                    />
                  </div>

                  <div className="label">Windows method</div>
                  <div>
                    <select
                      value={
                        (settingsDraft ?? data.settings).arma3.windows.method
                      }
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.arma3.windows.method = e.target
                          .value as WindowsLaunchMethod;
                        setSettingsDraft(next);
                      }}
                    >
                      <option value="steam_uri">steam_uri</option>
                      <option value="steam_app_launch">steam_app_launch</option>
                      <option value="direct_exe">direct_exe</option>
                    </select>
                  </div>

                  <div className="label">Windows Arma3 exe</div>
                  <div>
                    <input
                      placeholder="C:\\...\\Arma3_x64.exe"
                      value={
                        (settingsDraft ?? data.settings).arma3.windows
                          .arma3_exe ?? ""
                      }
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.arma3.windows.arma3_exe = e.target.value.trim()
                          ? e.target.value
                          : null;
                        setSettingsDraft(next);
                      }}
                    />
                  </div>

                  <div className="label">Windows Steam exe</div>
                  <div>
                    <input
                      placeholder="C:\\...\\Steam.exe"
                      value={
                        (settingsDraft ?? data.settings).arma3.windows
                          .steam_exe ?? ""
                      }
                      onChange={(e) => {
                        const next = cloneSettings(
                          settingsDraft ?? data.settings,
                        );
                        next.arma3.windows.steam_exe = e.target.value.trim()
                          ? e.target.value
                          : null;
                        setSettingsDraft(next);
                      }}
                    />
                  </div>
                </div>

                <div className="hr" />

                <div className="row">
                  <button
                    onClick={validateLinuxTemplate}
                    disabled={!activeProfile || !settingsDraft}
                  >
                    Validate Linux template (draft)
                  </button>
                  <button
                    onClick={() => {
                      if (!activeProfile) return;
                      void dm.requestLinuxValidation(activeProfile.id);
                    }}
                    disabled={!activeProfile}
                  >
                    Validate Linux template (saved)
                  </button>
                </div>

                {data?.linuxValidationError && (
                  <div className="error">{data.linuxValidationError}</div>
                )}
                {data?.linuxValidation && (
                  <pre className="log-viewer" style={{ maxHeight: 180 }}>
                    {JSON.stringify(data.linuxValidation, null, 2)}
                  </pre>
                )}
              </>
            )}
          </section>

          {/* Updates */}
          <section className="card">
            <div className="row space">
              <h2>Updates</h2>
              <div className="row">
                <button
                  onClick={() => updater.check()}
                  disabled={!updateState || updateState.type === "checking"}
                >
                  Check
                </button>
                <button
                  onClick={() => updater.clearError()}
                  disabled={!updateState || updateState.type !== "failed"}
                >
                  Clear error
                </button>
                <button
                  className="primary"
                  onClick={() => updater.apply()}
                  disabled={
                    !updateAvailable ||
                    !updateState ||
                    updateState.type === "downloading" ||
                    updateState.type === "checking"
                  }
                >
                  Install
                </button>
              </div>
            </div>

            {!updateState ? (
              <div className="muted">Awaiting updater…</div>
            ) : (
              <>
                <div className="row">
                  <span className="badge">State: {updateState.type}</span>
                  {updateState.type === "idle" && (
                    <span className="muted">{updateState.status}</span>
                  )}
                  {updateState.type === "failed" && (
                    <span className="error">{updateState.error}</span>
                  )}
                  {updateState.type === "downloading" && (
                    <span className="muted">
                      Downloading…{" "}
                      {updateState.progress != null
                        ? `${Math.round(updateState.progress * 100)}%`
                        : ""}
                    </span>
                  )}
                  {updateState.type === "notConfigured" && (
                    <span className="muted">Update feed not configured.</span>
                  )}
                </div>

                {updateAvailable && (
                  <pre className="log-viewer" style={{ maxHeight: 160 }}>
                    {JSON.stringify(updateAvailable, null, 2)}
                  </pre>
                )}
              </>
            )}
          </section>
        </main>
      </div>

      {/* Profile modal (simple inline) */}
      {profileDraft && (
        <div className="card" style={{ margin: 14 }}>
          <div className="row space">
            <h3>{isEditingProfile ? "Edit profile" : "Create profile"}</h3>
            <div className="row">
              <button onClick={closeProfileModal}>Close</button>
              <button className="primary" onClick={submitProfile}>
                Save
              </button>
            </div>
          </div>

          <div className="form-grid">
            <div className="label">Name</div>
            <div>
              <input
                value={profileDraft.name}
                onChange={(e) =>
                  setProfileDraft({ ...profileDraft, name: e.target.value })
                }
              />
            </div>

            <div className="label">Repo URL</div>
            <div>
              <input
                value={profileDraft.repo_url}
                onChange={(e) =>
                  setProfileDraft({ ...profileDraft, repo_url: e.target.value })
                }
              />
            </div>

            <div className="label">Checkout path</div>
            <div>
              <input
                value={profileDraft.checkout_root}
                onChange={(e) =>
                  setProfileDraft({
                    ...profileDraft,
                    checkout_root: e.target.value,
                  })
                }
              />
            </div>

            <div className="label">Select</div>
            <div className="row">
              <input
                type="checkbox"
                checked={profileDraft.select}
                onChange={(e) =>
                  setProfileDraft({ ...profileDraft, select: e.target.checked })
                }
              />
              <span className="small">
                Make this the active profile after save
              </span>
            </div>

            <div className="label">Arma 3 extra args</div>
            <div>
              <input
                value={profileDraft.arma3_extra_args}
                onChange={(e) =>
                  setProfileDraft({
                    ...profileDraft,
                    arma3_extra_args: e.target.value,
                  })
                }
              />
            </div>

            <div className="label">Enabled mods</div>
            <div>
              <textarea
                value={profileDraft.arma3_enabled_mods_csv}
                onChange={(e) =>
                  setProfileDraft({
                    ...profileDraft,
                    arma3_enabled_mods_csv: e.target.value,
                  })
                }
                placeholder="@cba_a3, @ace, @rhsusf..."
              />
              <div className="small">
                Comma-separated mod folder names (e.g. @ace).
              </div>
            </div>
          </div>

          {uiError && (
            <div className="error" style={{ marginTop: 10 }}>
              {uiError}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
