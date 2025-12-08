use camino::Utf8Path;
use fleet_infra::hashing::scan_file;
use std::fs;

// This test verifies that sorting PBO entries produces the expected checksum
#[test]
fn pbo_entries_sorted_produces_expected_checksum() {
    // Path to test files (relative to workspace root). Use CARGO_MANIFEST_DIR
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let mut base_path = std::path::PathBuf::from(&manifest_dir);
    base_path.push("..");
    base_path.push("..");
    base_path.push("..");
    base_path.push("test_files");
    base_path.push("@ace_compat_cup_vehicles");
    let base_path = base_path
        .canonicalize()
        .expect("failed to canonicalize base test_files path");
    let base = Utf8Path::from_path(&base_path).expect("invalid utf8 in base path");

    // Read mod.srf to get the expected MD5 checksum for the PBO (mod.srf stores MD5)
    let mod_srf_path = base.join("mod.srf");
    let mod_srf = fs::read_to_string(mod_srf_path.as_str()).expect("failed to read mod.srf");

    // Locate the Addons/cup_vehicles_ace_compat.pbo entry and extract the "Checksum":"..." value
    let needle = "addons/cup_vehicles_ace_compat.pbo";
    let mut expected: Option<String> = None;
    if let Some(pos) = mod_srf.find(needle) {
        if let Some(chk_pos) = mod_srf[pos..].find("Checksum\":\"") {
            let start = pos + chk_pos + "Checksum\":\"".len();
            if let Some(end) = mod_srf[start..].find('"') {
                let chk = &mod_srf[start..start + end];
                expected = Some(chk.to_string());
            }
        }
    }

    let expected = expected.expect(
        "expected MD5 checksum for Addons/cup_vehicles_ace_compat.pbo not found in mod.srf",
    );

    let fs_path = base.join("addons").join("cup_vehicles_ace_compat.pbo");
    let logical = Utf8Path::new("Addons/cup_vehicles_ace_compat.pbo");

    let file = scan_file(&fs_path, logical).expect("scan_file failed");

    assert_eq!(file.checksum.to_uppercase(), expected.to_uppercase());
}
