import { useEffect, useState } from "react";
import {
  DataModel,
  SyncReadModel,
  UpdateModel,
  dataSnapshot,
  subscribeSyncState,
  subscribeUpdateState,
  syncStart,
  syncCancel,
  updateCheck,
} from "./api";

import "./App.css";

export default function App() {
  const [data, setData] = useState<DataModel | null>(null);
  const [sync, setSync] = useState<SyncReadModel | null>(null);
  const [update, setUpdate] = useState<UpdateModel | null>(null);

  useEffect(() => {
    // Initial fetch
    dataSnapshot().then(setData).catch(console.error);
    
    // Subscriptions
    const subSync = subscribeSyncState(setSync);
    const subUpdate = subscribeUpdateState(setUpdate);

    return () => {
      // Cleanup if subscriptions supported cancellation tokens or similar
      void subSync;
      void subUpdate;
    };
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
            <button onClick={() => updateCheck()}>Check Updates</button>
          </div>
        ) : (
          <p>Waiting for update service...</p>
        )}
      </div>
    </div>
  );
}
