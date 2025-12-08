use fleet_core::diff::diff;
use fleet_core::{File, FileType, Manifest, Mod};

// --- Helper Functions to build Manifests easily ---

fn make_file(path: &str, checksum: &str) -> File {
    File {
        path: path.to_string(),
        length: 100,
        checksum: checksum.to_string(),
        file_type: FileType::File,
        parts: vec![],
    }
}

fn make_mod(name: &str, files: Vec<File>) -> Mod {
    Mod {
        name: name.to_string(),
        checksum: "mod_hash".to_string(),
        files,
    }
}

fn make_manifest(mods: Vec<Mod>) -> Manifest {
    Manifest {
        version: "1.0".to_string(),
        mods,
    }
}

// --- Tests ---

#[test]
fn test_case_mismatch_rename() {
    let local = make_manifest(vec![make_mod(
        "@ACE",
        vec![make_file("addons/main.pbo", "hash1")],
    )]);

    let remote = make_manifest(vec![make_mod(
        "@ace",
        vec![make_file("addons/main.pbo", "hash1")],
    )]);

    let plan = diff(&remote, &local);

    assert_eq!(plan.renames.len(), 1, "Should have 1 rename action");
    let rename = &plan.renames[0];
    assert_eq!(rename.old_path, "@ACE");
    assert_eq!(rename.new_path, "@ace");

    assert!(plan.deletes.is_empty(), "Should be no mod deletes");
    assert!(plan.downloads.is_empty(), "Should be no downloads");
    assert_eq!(plan.checks.len(), 1, "Should verify the existing file");
}

#[test]
fn test_collision_resolution_exact_match_wins() {
    let local = make_manifest(vec![
        make_mod("@ACE", vec![make_file("addons/old.pbo", "hash1")]),
        make_mod("@ace", vec![make_file("addons/main.pbo", "hash2")]),
    ]);

    let remote = make_manifest(vec![make_mod(
        "@ace",
        vec![make_file("addons/main.pbo", "hash2")],
    )]);

    let plan = diff(&remote, &local);

    assert_eq!(plan.deletes.len(), 1);
    assert_eq!(plan.deletes[0].path, "@ACE");

    assert!(
        plan.renames.is_empty(),
        "Survivor matched exactly, no rename needed"
    );

    assert_eq!(plan.checks.len(), 1);
    assert_eq!(plan.checks[0].path, "@ace/addons/main.pbo");
}

#[test]
fn test_collision_resolution_arbitrary_wins() {
    let local = make_manifest(vec![
        make_mod("@ACE", vec![make_file("addons/main.pbo", "hash1")]),
        make_mod("@Ace", vec![make_file("addons/main.pbo", "hash1")]),
    ]);

    let remote = make_manifest(vec![make_mod(
        "@ace",
        vec![make_file("addons/main.pbo", "hash1")],
    )]);

    let plan = diff(&remote, &local);

    assert_eq!(
        plan.deletes.len(),
        1,
        "One duplicate folder must be deleted"
    );
    let deleted_path = &plan.deletes[0].path;

    assert_eq!(plan.renames.len(), 1, "Survivor must be renamed");
    let rename = &plan.renames[0];

    assert_ne!(
        deleted_path, &rename.old_path,
        "Cannot delete and rename the same folder"
    );
    assert_eq!(rename.new_path, "@ace");
}

#[test]
fn test_orphan_cleanup() {
    let local = make_manifest(vec![make_mod("@Unused", vec![])]);
    let remote = make_manifest(vec![]);

    let plan = diff(&remote, &local);

    assert_eq!(plan.deletes.len(), 1);
    assert_eq!(plan.deletes[0].path, "@Unused");
}

#[test]
fn test_mixed_grand_harmonization() {
    let local = make_manifest(vec![
        make_mod("@A", vec![make_file("f1", "h1")]),
        make_mod("@B", vec![make_file("f2", "h2")]),
        make_mod("@C", vec![make_file("f3", "h3")]),
        make_mod("@c", vec![make_file("f3", "h3")]),
        make_mod("@D", vec![make_file("f4", "h4")]),
    ]);

    let remote = make_manifest(vec![
        make_mod("@A", vec![make_file("f1", "h1")]),
        make_mod("@b", vec![make_file("f2", "h2")]),
        make_mod("@c", vec![make_file("f3", "h3")]),
    ]);

    let plan = diff(&remote, &local);

    let deleted_paths: Vec<&String> = plan.deletes.iter().map(|d| &d.path).collect();
    assert!(
        deleted_paths.contains(&&"@D".to_string()),
        "Orphan @D missing"
    );
    assert!(
        deleted_paths.contains(&&"@C".to_string()),
        "Victim @C missing"
    );
    assert_eq!(plan.deletes.len(), 2);

    assert_eq!(plan.renames.len(), 1);
    assert_eq!(plan.renames[0].old_path, "@B");
    assert_eq!(plan.renames[0].new_path, "@b");

    let check_paths: Vec<&String> = plan.checks.iter().map(|c| &c.path).collect();
    assert!(check_paths.iter().any(|p| p.starts_with("@A/")), "Check @A");
    assert!(check_paths.iter().any(|p| p.starts_with("@c/")), "Check @c");
    assert!(check_paths.iter().any(|p| p.starts_with("@B/")), "Check @B");
}

#[test]
fn test_file_deletion_uses_old_mod_name() {
    let local = make_manifest(vec![make_mod(
        "@ACE",
        vec![make_file("kept.pbo", "h1"), make_file("unused.pbo", "h2")],
    )]);

    let remote = make_manifest(vec![make_mod("@ace", vec![make_file("kept.pbo", "h1")])]);

    let plan = diff(&remote, &local);

    assert_eq!(plan.renames.len(), 1);
    assert_eq!(plan.renames[0].old_path, "@ACE");

    assert_eq!(plan.deletes.len(), 1);
    assert_eq!(plan.deletes[0].path, "@ACE/unused.pbo");
}
