use fleet_core::{is_destination_unique, validate_profile_name, validate_repo_url, AppState};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileDraft {
    pub name: String,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileDraftValidation {
    pub name_ok: bool,
    pub repo_ok: bool,
    pub folder_ok: bool,
}

impl ProfileDraftValidation {
    pub fn is_valid(self) -> bool {
        self.name_ok && self.repo_ok && self.folder_ok
    }
}

impl ProfileDraft {
    pub fn from_fields(
        name: impl Into<String>,
        source: impl Into<String>,
        destination: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            destination: destination.into(),
        }
    }

    pub fn trimmed(&self) -> Self {
        Self {
            name: self.name.trim().to_string(),
            source: self.source.trim().to_string(),
            destination: self.destination.trim().to_string(),
        }
    }

    pub fn validate(&self, state: &AppState, ignore_id: Option<&str>) -> ProfileDraftValidation {
        let name_ok = validate_profile_name(&self.name);
        let repo_ok = self.source.trim().is_empty() || validate_repo_url(&self.source);
        let folder_ok = !self.destination.trim().is_empty()
            && is_destination_unique(state, &self.destination, ignore_id);
        ProfileDraftValidation {
            name_ok,
            repo_ok,
            folder_ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ProfileDraft;

    #[test]
    fn validate_accepts_valid_minimal_profile_draft() {
        let mut state = fleet_core::AppState::default();
        state.profiles.insert(
            "other".to_string(),
            fleet_core::Profile {
                id: "other".to_string(),
                name: "Other".to_string(),
                source: "https://example.com/repo.json".to_string(),
                destination: "/tmp/other".to_string(),
                ..Default::default()
            },
        );
        let draft =
            ProfileDraft::from_fields("Alpha One", "https://example.com/repo.json", "/tmp/alpha");
        assert!(draft.validate(&state, None).is_valid());
    }
}
