use camino::Utf8PathBuf;
use fleet_scan::{scan_mod, ScanOptions};
use std::path::PathBuf;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test_files")
}

#[test]
fn scan_mod_is_deterministic_and_orders_by_rel_path_and_excludes_mod_srf() {
    let mod_root = fixture_root().join("@ace_compat_cup_vehicles");
    let mod_root_utf8 =
        Utf8PathBuf::from_path_buf(mod_root.clone()).expect("fixture path should be utf8");

    let manifest = scan_mod(
        &mod_root_utf8,
        "@ace_compat_cup_vehicles",
        ScanOptions::default(),
    )
    .expect("scan fixture");

    let paths: Vec<String> = manifest
        .files()
        .iter()
        .map(|f| f.rel_path().as_str().to_string())
        .collect();
    assert!(
        !paths.iter().any(|p| p.eq_ignore_ascii_case("mod.srf")),
        "scan should exclude mod.srf"
    );

    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "scan output is not lexicographically sorted");
}
