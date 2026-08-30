use crate::ProfileCommands;
use fleet_core::Core;

pub async fn run(core: &Core, command: ProfileCommands) -> anyhow::Result<()> {
    match command {
        ProfileCommands::List => {
            let cfg = core.list_profiles().await?;
            println!("{:<20} {:<20} {:<30}", "ID", "NAME", "SOURCE");
            for p in cfg.profiles {
                println!("{:<20} {:<20} {:<30}", p.id, p.name, p.source);
            }
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
            let saved = core
                .profile_save(profile)
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
            println!("Profile '{}' created.", saved.id);
        }
        ProfileCommands::Remove { name } => {
            core.profile_delete(name.clone())
                .await
                .map_err(|error| anyhow::anyhow!("{}: {}", error.code, error.message))?;
            println!("Profile '{}' removed.", name);
        }
    }
    Ok(())
}
