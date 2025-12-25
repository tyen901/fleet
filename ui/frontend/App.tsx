import { useEffect, useState } from "react";
import { DataModel, dataSnapshot, syncStart, syncCancel } from "./api";
import { useSyncJob } from "./app/model/useSyncJob";
import { useSyncLog } from "./app/model/useSyncLog";
import { useUpdater } from "./app/model/useUpdater";

import "./App.css";

export default function App() {
  const [data, setData] = useState<DataModel | null>(null);
  const sync = useSyncJob();
  const logs = useSyncLog(Boolean(sync));
  const updater = useUpdater();
  const update = updater.model;

  useEffect(() => {
    // Initial fetch
    dataSnapshot().then(setData).catch(console.error);
    
  }, []);

  return (
    <div className="container">
      <h1>Fleet UI</h1>
      
      <div className="card">
        <h2>Profile Data</h2>
        {data ? (
          <div>
            <p><strong>Selected:</strong> {data.selected_id ?? "None"}</p>
            <ul>
              {data.profiles.map(p => (
                <li key={p.id}>{p.name} ({p.repo_url})</li>
              ))}
            </ul>
          </div>
        ) : (
          <p>Loading data...</p>
        )}
      </div>

      <div className="card">
        <h2>Sync Status</h2>
        {sync ? (
          <div>
             <div className="status-row">
               <span>{sync.phase}</span>
               <span>{sync.percent}%</span>
             </div>
             <div className="progress-bar">
               <div className="fill" style={{ width: `${sync.percent}%` }}></div>
             </div>
             <div className="controls">
               <button onClick={() => syncStart("repair")} disabled={!sync.can_start}>
                 Repair
               </button>
               <button onClick={() => syncCancel()} disabled={!sync.can_cancel}>
                 Cancel
               </button>
             </div>
             {sync.error && <p className="error">{sync.error}</p>}
          </div>
        ) : (
          <p>Waiting for sync service...</p>
        )}
      </div>

      <div className="card">
        <h2>Update Status</h2>
        {update ? (
          <div>
            <p>State: {JSON.stringify(update.state)}</p>
            <div className="controls">
              <button onClick={() => updater.check()}>Check Updates</button>
              <button
                onClick={() => updater.apply()}
                disabled={!update.available}
              >
                Apply Update
              </button>
            </div>
          </div>
        ) : (
          <p>Waiting for update service...</p>
        )}
      </div>

      <div className="card">
        <h2>Sync Logs</h2>
        {logs.length > 0 ? (
          <div className="log-window">
            {logs.map((entry) => (
              <div key={entry.seq}>
                <strong>[{entry.level}]</strong> {entry.message}
              </div>
            ))}
          </div>
        ) : (
          <p>No log entries yet.</p>
        )}
      </div>
    </div>
  );
}
