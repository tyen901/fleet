use anyhow::{Context, Result};

use crate::RepoSpec;

pub fn parse(bytes: &[u8]) -> Result<RepoSpec> {
    serde_json::from_slice(bytes).context("parse repo.json")
}
