use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use fleet_app_core::domain::{FLATPAK_STEAM_LAUNCH_TEMPLATE, STEAM_LAUNCH_TEMPLATE};
use fleet_cli::{commands, profiles, CliScanStrategy, CliSyncMode};
use fleet_infra::launcher::Launcher;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage profiles (saved configurations)
    Profile {
        #[command(subcommand)]
        command: ProfileCommands,
    },
    Scan {
        path: Utf8PathBuf,
        #[arg(short, long)]
        output: Option<Utf8PathBuf>,
        #[arg(long, value_enum, default_value_t = CliScanStrategy::Smart)]
        strategy: CliScanStrategy,
    },
    #[command(name = "check-for-updates", alias = "check")]
    CheckForUpdates {
        #[arg(long, required_unless_present = "profile")]
        repo: Option<String>,
        #[arg(long, required_unless_present = "profile")]
        path: Option<Utf8PathBuf>,
        #[arg(short, long, help = "Use settings from a named profile")]
        profile: Option<String>,
    },
    #[command(name = "local-check")]
    LocalCheck {
        #[arg(long, required_unless_present = "profile")]
        path: Option<Utf8PathBuf>,
        #[arg(short, long, help = "Use settings from a named profile")]
        profile: Option<String>,
    },
    Repair {
        #[arg(long, required_unless_present = "profile")]
        repo: Option<String>,
        #[arg(long, required_unless_present = "profile")]
        path: Option<Utf8PathBuf>,
        #[arg(short, long, help = "Use settings from a named profile")]
        profile: Option<String>,
    },
    Sync {
        #[arg(long, required_unless_present = "profile")]
        repo: Option<String>,
        #[arg(long, required_unless_present = "profile")]
        path: Option<Utf8PathBuf>,
        #[arg(short, long, help = "Use settings from a named profile")]
        profile: Option<String>,
        #[arg(long, value_enum, default_value_t = CliSyncMode::Smart)]
        mode: CliSyncMode,
        #[arg(short, long, default_value_t = 4)]
        threads: usize,
        #[arg(long)]
        limit_mb: Option<u64>,
        #[arg(long)]
        cache_dir: Option<Utf8PathBuf>,
    },
    Launch {
        #[arg(
            short,
            long,
            value_delimiter = ',',
            help = "Explicit mod list (overrides profile)"
        )]
        mods: Option<Vec<Utf8PathBuf>>,
        #[arg(short, long, help = "Use mods from a named profile")]
        profile: Option<String>,
        #[arg(long)]
        connect: Option<String>,
        #[arg(long, requires = "connect")]
        password: Option<String>,
        #[arg(long, default_value = "-noSplash -skipIntro -noLauncher")]
        args: String,
        #[arg(long, help = "Launch via Flatpak Steam (com.valvesoftware.Steam)")]
        flatpak: bool,
        #[arg(long, help = "Custom launch template (overrides default/--flatpak)")]
        template: Option<String>,
    },
}

#[derive(Subcommand)]
enum ProfileCommands {
    List,
    Add {
        #[arg(long, help = "Unique slug ID for the profile")]
        id: String,
        name: String,
        repo: String,
        path: Utf8PathBuf,
    },
    Remove {
        name: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let level = if cli.verbose {
        Level::DEBUG
    } else {
        Level::WARN
    };
    let subscriber = FmtSubscriber::builder().with_max_level(level).finish();
    tracing::subscriber::set_global_default(subscriber).expect("default subscriber");

    let resolve_profile = |name: &str| -> anyhow::Result<(String, Utf8PathBuf)> {
        let mgr = profiles::ProfileManager::new();
        let p = mgr.find(name)?;
        Ok((p.repo_url, Utf8PathBuf::from(p.local_path)))
    };

    match cli.command {
        Commands::Profile { command } => match command {
            ProfileCommands::List => profiles::handle_list()?,
            ProfileCommands::Add {
                id,
                name,
                repo,
                path,
            } => profiles::handle_add(id, name, repo, path)?,
            ProfileCommands::Remove { name } => profiles::handle_remove(name)?,
        },
        Commands::Scan {
            path,
            output,
            strategy,
        } => commands::cmd_scan(path, output, strategy).await?,
        Commands::CheckForUpdates {
            repo,
            path,
            profile,
        } => {
            let (final_repo, final_path) = if let Some(p_name) = profile {
                resolve_profile(&p_name)?
            } else {
                (repo.unwrap(), path.unwrap())
            };
            commands::cmd_check_for_updates(final_repo, final_path).await?;
        }
        Commands::LocalCheck { path, profile } => {
            let final_path = if let Some(p_name) = profile {
                let (_repo, path) = resolve_profile(&p_name)?;
                path
            } else {
                path.unwrap()
            };
            commands::cmd_local_check(final_path).await?;
        }
        Commands::Repair {
            repo,
            path,
            profile,
        } => {
            let (final_repo, final_path) = if let Some(p_name) = profile {
                resolve_profile(&p_name)?
            } else {
                (repo.unwrap(), path.unwrap())
            };
            commands::cmd_repair(final_repo, final_path).await?;
        }
        Commands::Sync {
            repo,
            path,
            profile,
            mode,
            threads,
            limit_mb,
            cache_dir,
        } => {
            let (final_repo, final_path) = if let Some(p_name) = profile {
                resolve_profile(&p_name)?
            } else {
                (repo.unwrap(), path.unwrap())
            };
            commands::cmd_sync(final_repo, final_path, mode, threads, limit_mb, cache_dir).await?;
        }
        Commands::Launch {
            mods,
            profile,
            connect,
            password,
            mut args,
            flatpak,
            template,
        } => {
            #[cfg(not(target_os = "linux"))]
            if flatpak {
                anyhow::bail!("--flatpak is only supported on Linux");
            }

            if let Some(addr) = connect {
                let (ip, port) = if let Some((host, p)) = addr.split_once(':') {
                    (host.to_string(), p.to_string())
                } else {
                    (addr, "2302".to_string())
                };
                args.push_str(&format!(" -connect={} -port={}", ip, port));
                if let Some(pwd) = password {
                    args.push_str(&format!(" -password={}", pwd));
                }
            }

            let launch_template = template.unwrap_or_else(|| {
                if flatpak {
                    FLATPAK_STEAM_LAUNCH_TEMPLATE.to_string()
                } else {
                    STEAM_LAUNCH_TEMPLATE.to_string()
                }
            });

            let launcher = Launcher::new("".to_string(), args, launch_template);

            let final_mods = if let Some(explicit_mods) = mods {
                explicit_mods
            } else if let Some(p_name) = profile {
                let mgr = profiles::ProfileManager::new();
                let p = mgr.find(&p_name)?;
                commands::resolve_mods_from_dir(&Utf8PathBuf::from(p.local_path))?
            } else {
                Vec::new()
            };

            launcher.launch(final_mods)?;
        }
    }

    Ok(())
}
