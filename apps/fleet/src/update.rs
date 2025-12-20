use clap::Parser;

pub const DEFAULT_GITHUB_LATEST_SUFFIX: &str = "/releases/latest/download";

#[derive(Parser, Debug, Clone)]
pub struct UpdateArgs {
    /// Override the update feed base URL.
    /// Must be a base URL such that `{baseUrl}/RELEASES` exists.
    ///
    /// Example (GitHub Releases):
    ///   https://github.com/<owner>/<repo>/releases/latest/download
    #[arg(long)]
    pub url: Option<String>,

    /// Only check and print status; do not download/apply.
    #[arg(long)]
    pub check: bool,

    /// Print JSON (when applicable).
    #[arg(long)]
    pub json: bool,
}

fn normalize_base_url(s: String) -> String {
    let mut t = s.trim().to_string();
    while t.ends_with('/') {
        t.pop();
    }
    t
}

fn configured_base_url(override_url: Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(u) = override_url {
        let u = normalize_base_url(u);
        if !u.is_empty() {
            return Ok(u);
        }
    }

    if let Ok(u) = std::env::var("FLEET_UPDATE_URL") {
        let u = normalize_base_url(u);
        if !u.is_empty() {
            return Ok(u);
        }
    }

    if let Some(u) = option_env!("FLEET_UPDATE_URL") {
        let u = normalize_base_url(u.to_string());
        if !u.is_empty() {
            return Ok(u);
        }
    }

    if let Some(repo) = option_env!("CARGO_PKG_REPOSITORY") {
        let repo = repo.trim();
        if repo.starts_with("https://github.com/") && !repo.is_empty() {
            let derived = format!(
                "{}{}",
                repo.trim_end_matches('/'),
                DEFAULT_GITHUB_LATEST_SUFFIX
            );
            return Ok(derived);
        }
    }

    Err("Update feed URL not configured. Set FLEET_UPDATE_URL (compile-time or runtime), or pass --url.\nExample: https://github.com/<owner>/<repo>/releases/latest/download"
        .into())
}

pub fn run(args: &UpdateArgs) -> Result<(), Box<dyn std::error::Error>> {
    use velopack::{sources, UpdateCheck, UpdateManager};

    let base_url = configured_base_url(args.url.clone())?;

    let source = sources::HttpSource::new(&base_url);
    let um = UpdateManager::new(source, None, None)?;

    let check = um.check_for_updates()?;
    match check {
        UpdateCheck::RemoteIsEmpty | UpdateCheck::NoUpdateAvailable => {
            if args.json {
                println!(r#"{{"status":"no_update"}}"#);
            } else {
                println!("No update available.");
            }
            Ok(())
        }
        UpdateCheck::UpdateAvailable(info) => {
            if args.check {
                if args.json {
                    println!("{}", serde_json::to_string_pretty(&info)?);
                } else {
                    println!(
                        "Update available:\n{}",
                        serde_json::to_string_pretty(&info)?
                    );
                }
                return Ok(());
            }

            let (ptx, prx) = std::sync::mpsc::channel::<i16>();
            std::thread::spawn(move || {
                for p in prx {
                    let p = (p as i32).clamp(0, 100);
                    eprintln!("Downloading: {p}%");
                }
            });

            um.download_updates(&info, Some(ptx))?;
            eprintln!("Download complete. Applying update and exiting...");

            um.apply_updates_and_exit(&info)?;
            Ok(())
        }
    }
}
