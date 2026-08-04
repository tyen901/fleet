fn main() {
    // Stamp the build with tag/hash/date so shipped binaries can report exactly what they are.
    // This is best-effort and falls back to "unknown" when git isn't available.
    stamp_build_info();
}

fn stamp_build_info() {
    use std::env;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    let git_tag_exact = env_non_empty("FLEET_BUILD_TAG")
        .or_else(|| {
            let ref_type = env_non_empty("GITHUB_REF_TYPE");
            let ref_name = env_non_empty("GITHUB_REF_NAME");
            match (ref_type.as_deref(), ref_name) {
                (Some("tag"), Some(name)) => Some(name),
                _ => None,
            }
        })
        .or_else(|| cmd(&["git", "describe", "--tags", "--exact-match", "HEAD"]));

    let git_describe = env_non_empty("FLEET_BUILD_DESCRIBE")
        .or_else(|| cmd(&["git", "describe", "--tags", "--always", "--dirty"]))
        .unwrap_or_else(|| "unknown".to_string());

    let commit_date = cmd(&["git", "show", "-s", "--format=%cI", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string());

    let build_unix_seconds: u64 = env_non_empty("SOURCE_DATE_EPOCH")
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });

    println!("cargo:rustc-env=FLEET_GIT_HASH={git_hash}");
    println!("cargo:rustc-env=FLEET_GIT_SHORT_HASH={git_short_hash}");
    println!(
        "cargo:rustc-env=FLEET_GIT_TAG_EXACT={}",
        git_tag_exact.unwrap_or_else(|| "unknown".to_string())
    );
    println!("cargo:rustc-env=FLEET_GIT_DESCRIBE={git_describe}");
    println!("cargo:rustc-env=FLEET_GIT_COMMIT_DATE={commit_date}");
    println!("cargo:rustc-env=FLEET_BUILD_UNIX_SECONDS={build_unix_seconds}");
}
