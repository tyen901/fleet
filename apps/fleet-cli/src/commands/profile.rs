use crate::ProfileCommands;
use fleet_core::Core;

use super::check::{print_check_report, run_check_report};

pub async fn run(core: &Core, command: ProfileCommands) -> anyhow::Result<()> {
    match command {
        ProfileCommands::List => {
            let cfg = core.list_profiles().await?;
            println!("{:<20} {:<20} {:<30}", "ID", "NAME", "SOURCE");
            for p in cfg.profiles {
                println!("{:<20} {:<20} {:<30}", p.id, p.name, p.source);
            }
        }
        ProfileCommands::Check { profile_id } => {
            let profile = core.load_profile(&profile_id).await?;
            let report = run_check_report(core, &profile_id, true).await?;
            print_check_report(&report, !profile.source.trim().is_empty());
        }
        ProfileCommands::Add {
            id,
            name,
            source,
            dest,
        } => {
            let profile = fleet_core::Profile {
                id: id.clone(),
                name,
                source,
                destination: dest,
                ..Default::default()
            };
            let saved = core.save_profile(profile).await?;
            println!("Profile '{}' created.", saved.id);
        }
        ProfileCommands::Remove { name } => {
            core.delete_profile(&name).await?;
            println!("Profile '{}' removed.", name);
        }
    }
    Ok(())
}
