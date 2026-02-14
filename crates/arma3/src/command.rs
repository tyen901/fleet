use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMethod {
    /// Run Arma directly from the configured install path (prefer `arma3_x64.exe`).
    Arma3Exe,
    /// `steam -applaunch 107410 -nolauncher ...`
    SteamNative,
}

#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub method: LaunchMethod,
    pub executable: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

impl LaunchCommand {
    pub fn to_std_command(&self) -> Result<Command> {
        let mut cmd = Command::new(&self.executable);
        cmd.args(&self.args);

        for (k, v) in &self.env {
            cmd.env(k, v);
        }

        // Match HEMTT's behavior of not inheriting steam output.
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::null());

        Ok(cmd)
    }
}
