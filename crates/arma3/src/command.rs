use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMethod {
    /// Run Arma directly from the configured install path (prefer `arma3_x64.exe`).
    Arma3Exe,
    /// `steam -applaunch 107410 -nolauncher ...`
    SteamNative,
}

#[derive(Debug, Clone)]
pub struct LaunchCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl LaunchCommand {
    pub fn spawn(&self) -> std::io::Result<std::process::Child> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        cmd.spawn()
    }
}
