use fleet_index::FleetIndex;

pub(crate) struct FleetIndexStore {
    _lock: std::fs::File,
    inner: std::sync::Mutex<FleetIndex>,
}

impl FleetIndexStore {
    pub(crate) fn new(lock: std::fs::File, idx: FleetIndex) -> Self {
        Self {
            _lock: lock,
            inner: std::sync::Mutex::new(idx),
        }
    }
}

impl fleet_sync::StateStore for FleetIndexStore {
    fn desired_state_get(
        &self,
    ) -> Result<Option<fleet_sync::DesiredState>, fleet_sync::StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .get_desired_state()
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))?;
        Ok(got.map(|s| fleet_sync::DesiredState {
            state_id: s.state_id,
            enabled_mods_hash: s.enabled_mods_hash,
        }))
    }

    fn expected_replace_all_if_digest_changed(
        &self,
        state_id: &str,
        rows: Vec<fleet_sync::ExpectedFile>,
        digest_hex: &str,
    ) -> Result<(), fleet_sync::StoreError> {
        let rows: Vec<fleet_index::ExpectedFile> = rows
            .into_iter()
            .map(|r| fleet_index::ExpectedFile {
                mod_id: r.mod_id,
                rel_path: r.rel_path,
                size: r.size,
            })
            .collect();
        self.inner
            .lock()
            .unwrap()
            .expected_replace_all_if_digest_changed(state_id, rows, digest_hex)
            .map(|_| ())
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))
    }

    fn baseline_exists(&self, state_id: &str) -> Result<bool, fleet_sync::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .baseline_exists(state_id)
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))
    }

    fn expected_get_all(
        &self,
        state_id: &str,
    ) -> Result<Vec<fleet_sync::ExpectedFile>, fleet_sync::StoreError> {
        let mut out = Vec::new();
        self.inner
            .lock()
            .unwrap()
            .expected_for_each(state_id, |row| {
                out.push(fleet_sync::ExpectedFile {
                    mod_id: row.mod_id,
                    rel_path: row.rel_path,
                    size: row.size,
                });
                Ok(())
            })
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))?;
        Ok(out)
    }

    fn file_state_get_all_for_mod(
        &self,
        state_id: &str,
        mod_id: &str,
    ) -> Result<std::collections::HashMap<String, fleet_sync::FileState>, fleet_sync::StoreError>
    {
        let got = self
            .inner
            .lock()
            .unwrap()
            .file_state_get_all_for_mod(state_id, mod_id)
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))?;
        Ok(got
            .into_iter()
            .map(|(k, v)| {
                (
                    k,
                    fleet_sync::FileState {
                        size: v.size,
                        mtime_ns: fleet_sync::TimestampNs(v.mtime_ns),
                        checksum: v.checksum,
                    },
                )
            })
            .collect())
    }

    fn file_state_apply_batch(
        &self,
        state_id: &str,
        upserts: Vec<fleet_sync::FileStateUpsert>,
        deletes: Vec<fleet_sync::FileStateDelete>,
    ) -> Result<(), fleet_sync::StoreError> {
        let up = upserts
            .into_iter()
            .map(|u| (u.mod_id, u.rel_path, u.size, u.mtime_ns.0, u.checksum));
        let del = deletes.into_iter().map(|d| (d.mod_id, d.rel_path));
        self.inner
            .lock()
            .unwrap()
            .file_state_apply_batch(state_id, up, del)
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))
    }

    fn file_state_delete(
        &self,
        state_id: &str,
        mod_id: &str,
        rel_path: &str,
    ) -> Result<(), fleet_sync::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .file_state_delete(state_id, mod_id, rel_path)
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))
    }

    fn verified_get(&self) -> Result<Option<fleet_sync::VerifiedState>, fleet_sync::StoreError> {
        let got = self
            .inner
            .lock()
            .unwrap()
            .verified_get()
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))?;
        Ok(got.map(|v| fleet_sync::VerifiedState {
            state_id: v.state_id,
            verified_at: fleet_sync::TimestampNs(v.verified_at_ns),
        }))
    }

    fn verified_set(
        &self,
        state_id: &str,
        verified_at: fleet_sync::TimestampNs,
    ) -> Result<(), fleet_sync::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_set(state_id, verified_at.0)
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))
    }

    fn verified_clear(&self) -> Result<(), fleet_sync::StoreError> {
        self.inner
            .lock()
            .unwrap()
            .verified_clear()
            .map_err(|e| fleet_sync::StoreError::Other(e.to_string()))
    }
}
