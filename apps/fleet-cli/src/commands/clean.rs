use fleet_core::{Core, LocalHealthState};

use super::flow_run::prompt_delete_confirmation;
use super::load_profile;

pub async fn run(core: &Core, profile_id: &str) -> anyhow::Result<()> {
    let profile = load_profile(core, profile_id).await?;

    let report = core
        .profile_check_with_intent(profile.id.clone(), false)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;

    let mut unexpected_paths = report.unexpected_delete_paths;
    unexpected_paths.sort();

    if unexpected_paths.is_empty() {
        println!("No unexpected files found.");
        return Ok(());
    }

    let prompt = format_clean_prompt(&unexpected_paths);
    let confirm = prompt_delete_confirmation(&prompt).await?;

    if !confirm {
        println!("Cleanup skipped; unexpected files remain.");
        return Ok(());
    }

    core.assessment_delete_extra_files(profile.id.clone())
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;

    let after = core
        .profile_check_with_intent(profile.id.clone(), false)
        .await
        .map_err(|e| anyhow::anyhow!("{}: {}", e.code, e.message))?;

    println!("---");
    println!("clean done");
    println!(
        "remaining_unexpected_files: {}",
        after.unexpected_delete_paths.len()
    );
    println!("local_health: {:?}", after.local_health);
    if after.local_health == LocalHealthState::LocalDrift
        && !after.unexpected_delete_paths.is_empty()
    {
        println!("Some unexpected files remain after cleanup.");
    }

    Ok(())
}

fn format_clean_prompt(paths: &[String]) -> String {
    let mut prompt = if paths.len() == 1 {
        "Delete 1 unexpected file?".to_string()
    } else {
        format!("Delete {} unexpected files?", paths.len())
    };

    for path in paths {
        prompt.push('\n');
        prompt.push_str("- ");
        prompt.push_str(path);
    }

    prompt
}

#[cfg(test)]
mod tests {
    use super::format_clean_prompt;

    #[test]
    fn format_clean_prompt_lists_paths() {
        let prompt = format_clean_prompt(&["extra.txt".to_string(), "mods/a.pbo".to_string()]);
        assert!(prompt.starts_with("Delete 2 unexpected files?"));
        assert!(prompt.contains("\n- extra.txt"));
        assert!(prompt.contains("\n- mods/a.pbo"));
    }
}
