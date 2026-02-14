fn main() {
    stamp_cli_version();
}

fn stamp_cli_version() {
    use std::env;
    use std::process::Command;

    fn env_non_empty(key: &str) -> Option<String> {
        env::var(key).ok().and_then(|v| {
            let s = v.trim().to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
    }

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

    let git_hash = env_non_empty("FLEET_BUILD_HASH")
        .or_else(|| env_non_empty("GITHUB_SHA"))
        .or_else(|| cmd(&["git", "rev-parse", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());

    let git_short_hash = if git_hash != "unknown" && git_hash.len() >= 7 {
        git_hash[..7].to_string()
    } else {
        "unknown".to_string()
    };

    let tag = env_non_empty("FLEET_BUILD_TAG")
        .or_else(|| {
            let ref_type = env_non_empty("GITHUB_REF_TYPE");
            let ref_name = env_non_empty("GITHUB_REF_NAME");
            match (ref_type.as_deref(), ref_name) {
                (Some("tag"), Some(name)) => Some(name),
                _ => None,
            }
        })
        .or_else(|| cmd(&["git", "describe", "--tags", "--exact-match", "HEAD"]))
        .or_else(|| cmd(&["git", "describe", "--tags", "--always", "--dirty"]))
        .unwrap_or_else(|| "unknown".to_string());

    let version = if git_short_hash != "unknown" {
        format!("{tag} ({git_short_hash})")
    } else {
        tag
    };

    println!("cargo:rustc-env=FLEET_CLI_VERSION={version}");
}
