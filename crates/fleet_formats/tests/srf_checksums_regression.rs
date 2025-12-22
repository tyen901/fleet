use fleet_formats::{file_checksum_from_parts, mod_checksum_from_files};
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test_files")
}

fn read_fixture(rel: &str) -> Vec<u8> {
    std::fs::read(fixture_root().join(rel)).expect("read fixture file")
}

#[test]
fn srf_fixture_file_and_mod_checksums_match_swifty_rules() {
    let srf_bytes = read_fixture("@ace_compat_cup_vehicles/mod.srf");
    let manifest =
        fleet_formats::srf_json::parse_mod_manifest(&srf_bytes).expect("parse mod.srf fixture");

    for f in &manifest.files {
        let got = file_checksum_from_parts(&f.parts);
        assert_eq!(
            got, f.file_checksum,
            "file checksum mismatch for {}",
            f.rel_path
        );
    }

    let got_mod = mod_checksum_from_files(&manifest.files);
    assert_eq!(
        got_mod, manifest.checksum,
        "mod checksum mismatch for mod_id={}",
        manifest.mod_id
    );
}
