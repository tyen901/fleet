fn main() {
    stamp_cli_version();
}

fn stamp_cli_version() {
    use std::process::Command;

    fn cmd(args: &[&str]) -> Option<String> {
        let mut c = Command::new(args[0]);
        if args.len() > 1 {
            c.args(&args[1..]);
        }
        let out = c.output().ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    let git_hash =
        cmd(&["git", "rev-parse", "HEAD"]).expect("Git revision is required to build Fleet");

    let git_short_hash = &git_hash[..7];

    let tag = cmd(&["git", "describe", "--tags", "--always", "--dirty"])
        .expect("Git description is required to build Fleet");

    let version = format!("{tag} ({git_short_hash})");

    println!("cargo:rustc-env=FLEET_CLI_VERSION={version}");
}
