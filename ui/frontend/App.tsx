import { dataLaunchArma3, syncCancel, syncStart } from "./api";
import { useDataModel } from "./app/model/useDataModel";
import { useSyncJob } from "./app/model/useSyncJob";
import { useSyncLog } from "./app/model/useSyncLog";
import { useUpdater } from "./app/model/useUpdater";

import "./App.css";

export default function App() {
  const { data, selectProfile } = useDataModel();
  const sync = useSyncJob();
  const logs = useSyncLog(Boolean(sync && !sync.finished));
  const updater = useUpdater();

  const activeProfile = data?.profiles.find((p) => p.id === data.selected_id);

  const handleLaunch = async () => {
    if (activeProfile) {
      await dataLaunchArma3(activeProfile.id);
    }
  };

  const updateModel = updater.state;
  const updateState = updateModel?.state;
  const available = updateModel?.available;

  return (
    <div className="shell">
      <header className="app-header">
        <h1>FLEET</h1>
        <div className="version-tag">v0.1.0</div>
      </header>

      <div className="layout">
        <aside className="sidebar">
          <h3>Profiles</h3>
          <div className="profile-list">
            {data?.profiles.map((p) => (
              <button
                key={p.id}
                className={p.id === data.selected_id ? "active" : ""}
                onClick={() => selectProfile(p.id)}
              >
                {p.name}
              </button>
            ))}
            {!data && <div className="muted">Loading...</div>}
          </div>
        </aside>

        <main className="content">
          {activeProfile ? (
            <>
              <section className="card">
                <h2>{activeProfile.name}</h2>
                <div className="kv-grid">
                  <span className="label">Repository:</span>
                  <span className="value">{activeProfile.repo_url}</span>
                  <span className="label">Path:</span>
                  <span className="value">{activeProfile.checkout_root}</span>
                </div>

                <div className="actions">
                  <button
                    className="primary"
                    onClick={() => void handleLaunch()}
                    disabled={!sync?.finished}
                  >
                    Launch Arma 3
                  </button>
                </div>
              </section>

              <section className="card">
                <div className="card-head">
                  <h2>Sync Status</h2>
                  <div className="controls">
                    <button
                      onClick={() => syncStart("repair")}
                      disabled={!sync?.can_start}
                    >
                      Repair
                    </button>
                    <button
                      onClick={() => syncCancel()}
                      disabled={!sync?.can_cancel}
                      className="danger"
                    >
                      Cancel
                    </button>
                  </div>
                </div>

                <div className="status-box">
                  <div className="status-line">
                    {sync?.status_line ?? "Idle"}
                  </div>
                  <div className="progress-container">
                    <div
                      className="progress-bar"
                      style={{ width: `${sync?.percent ?? 0}%` }}
                    />
                    <span className="pct">{sync?.percent ?? 0}%</span>
                  </div>
                </div>

                <div className="log-viewer">
                  {logs.length > 0 ? (
                    logs.map((entry) => (
                      <div
                        key={entry.seq}
                        className={`log-line ${entry.level.toLowerCase()}`}
                      >
                        <span className="seq">[{entry.seq}]</span>{" "}
                        {entry.message}
                      </div>
                    ))
                  ) : (
                    <div className="muted">No recent logs.</div>
                  )}
                </div>
              </section>
            </>
          ) : (
            <div className="empty-state">Select a profile to begin.</div>
          )}

          <section className="card update-card">
            <h3>Updates</h3>
            {updateState ? (
              <>
                {updateState.type === "idle" && (
                  <div className="row">
                    <span>{updateState.status}</span>
                    <button onClick={() => updater.check()}>Check</button>
                    {available && (
                      <button
                        className="primary"
                        onClick={() => updater.apply()}
                      >
                        Install {available.TargetFullRelease.Version}
                      </button>
                    )}
                  </div>
                )}
                {updateState.type === "downloading" && (
                  <div className="status-line">Downloading update…</div>
                )}
                {updateState.type === "failed" && (
                  <div className="error">
                    Update failed: {updateState.error}
                  </div>
                )}
              </>
            ) : (
              <div className="muted">Awaiting updater…</div>
            )}
          </section>
        </main>
      </div>
    </div>
  );
}
