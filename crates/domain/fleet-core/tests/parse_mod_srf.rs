use fleet_core::formats::parse_srf;
use std::fs::read;
use std::path::PathBuf;

#[test]
fn parse_test_mod_srf() {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // tests run with manifest dir pointing at the crate; go up to workspace root
    path.pop(); // crates/infrastructure/fleet-formats
    path.pop(); // crates/infrastructure
    path.pop(); // crates
    path.push("test_files");
    path.push("mod.srf");

    let data =
        read(&path).unwrap_or_else(|_| panic!("failed to read test file: {}", path.display()));
    let parsed = parse_srf(&data).expect("failed to parse mod.srf");
    assert!(!parsed.name.is_empty());
    assert!(!parsed.files.is_empty());
}
