use crate::ProfileCommands;
use fleet_core::Core;

use super::check::{print_check_report, run_check};

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
            let report = run_check(core, &profile_id).await?;
            print_check_report(&report.repo, &report.local);
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
