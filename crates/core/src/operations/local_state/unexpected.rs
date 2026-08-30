use ignore::{gitignore::GitignoreBuilder, WalkBuilder};
use std::collections::BTreeSet;
use std::path::Path;
use tokio_util::sync::CancellationToken;

pub(crate) fn enumerate_unexpected_paths(
    root: &Path,
    manifest: &flux::ValidatedManifest,
    ignore_rules: &str,
    cancel: &CancellationToken,
) -> anyhow::Result<Vec<flux::TargetPath>> {
    let expected = manifest
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    let matcher = inline_ignore_matcher(root, ignore_rules)?;
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .hidden(false)
        .parents(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .ignore(true)
        .sort_by_file_path(|left, right| left.cmp(right));
    if let Some(matcher) = matcher {
        builder.filter_entry(move |entry| {
            !matcher
                .matched(
                    entry.path(),
                    entry.file_type().is_some_and(|kind| kind.is_dir()),
                )
                .is_ignore()
        });
    }

    let mut unexpected = Vec::new();
    for entry in builder.build() {
        if cancel.is_cancelled() {
            anyhow::bail!("canceled");
        }
        let entry = entry?;
        if entry.depth() == 0
            || !entry.file_type().is_some_and(|kind| kind.is_file())
            || entry.path_is_symlink()
        {
            continue;
        }
        let relative = entry.path().strip_prefix(root)?;
        let path = fleet_inventory::target_path_from_relative_path(relative)?;
        if !expected.contains(&path) {
            unexpected.push(path);
        }
    }
    unexpected.sort();
    Ok(unexpected)
}

fn inline_ignore_matcher(
    root: &Path,
    rules: &str,
) -> anyhow::Result<Option<ignore::gitignore::Gitignore>> {
    if rules.trim().is_empty() {
        return Ok(None);
    }
    let mut builder = GitignoreBuilder::new(root);
    for line in rules.trim().lines() {
        builder.add_line(None, line)?;
    }
    Ok(Some(builder.build()?))
}
