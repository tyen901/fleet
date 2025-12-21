use sync_engine::fetch::{FileEntry, FilePart, ModManifest};
use sync_engine::manifest::validate_and_normalize_manifest;

fn build_manifest(mod_id: &str, rel_path: &str, size: u64, parts: Vec<FilePart>) -> ModManifest {
    ModManifest {
        mod_id: mod_id.to_string(),
        files: vec![FileEntry {
            rel_path: rel_path.to_string(),
            size,
            file_checksum: vec![0; 16],
            parts,
        }],
    }
}

#[test]
fn manifest_rejects_invalid_mod_id() {
    let m = build_manifest("", "file.bin", 4, vec![]);
    assert!(validate_and_normalize_manifest(m).is_err());
}

#[test]
fn manifest_rejects_invalid_rel_path() {
    let m = build_manifest("@mod", "../file.bin", 4, vec![]);
    assert!(validate_and_normalize_manifest(m).is_err());
}

#[test]
fn manifest_rejects_overlapping_parts() {
    let m = build_manifest(
        "@mod",
        "file.bin",
        10,
        vec![
            FilePart {
                offset: 0,
                len: 6,
                checksum: vec![1],
            },
            FilePart {
                offset: 5,
                len: 3,
                checksum: vec![2],
            },
        ],
    );
    assert!(validate_and_normalize_manifest(m).is_err());
}

#[test]
fn manifest_rejects_part_out_of_bounds() {
    let m = build_manifest(
        "@mod",
        "file.bin",
        10,
        vec![FilePart {
            offset: 8,
            len: 5,
            checksum: vec![1],
        }],
    );
    assert!(validate_and_normalize_manifest(m).is_err());
}

#[test]
fn manifest_rejects_zero_length_part() {
    let m = build_manifest(
        "@mod",
        "file.bin",
        10,
        vec![FilePart {
            offset: 0,
            len: 0,
            checksum: vec![1],
        }],
    );
    assert!(validate_and_normalize_manifest(m).is_err());
}
