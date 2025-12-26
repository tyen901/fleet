use std::path::PathBuf;

use fleet_swifty_wire::{ingest_mod_srf, parse_mod_srf, parse_repo_spec_json};

fn fixtures_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn read_fixture(rel: &str) -> Vec<u8> {
    std::fs::read(fixtures_root().join(rel)).expect("read fixture file")
}

#[test]
fn parses_real_example_repo_json_fixture() {
    let bytes = read_fixture("example_repo.json");

    let spec = parse_repo_spec_json(&bytes).expect("parse_repo_spec_json(example_repo.json)");

    assert_eq!(spec.repo_name, "modpack_test");
    assert_eq!(spec.version, "3.2.0.0");
    assert_eq!(spec.client_parameters, "-skipIntro");
    assert_eq!(spec.required_mods.len(), 5);
    assert!(spec.optional_mods.is_empty());

    let auth = spec
        .repo_basic_authentication
        .expect("expected repoBasicAuthentication in fixture");
    assert_eq!(auth.username, "userName");
    assert_eq!(auth.password, "test");

    assert_eq!(spec.servers.len(), 1);
    assert_eq!(spec.servers[0].port, 3000);
}

#[test]
fn parses_and_ingests_real_example_mod_srf_fixture() {
    let bytes = read_fixture("example_mod.srf");
    assert_eq!(
        bytes.len(),
        420_665,
        "fixture size changed; update the asserted byte length if intentional"
    );

    let wire = parse_mod_srf(&bytes).expect("parse_mod_srf(example_mod.srf)");
    let domain = ingest_mod_srf(wire).expect("ingest_mod_srf(example_mod.srf)");

    let mod_id = domain.mod_id().as_str();
    assert!(!mod_id.trim().is_empty(), "expected non-empty mod id");
    assert!(
        mod_id.starts_with('@'),
        "expected mod id to start with '@', got {mod_id}"
    );

    assert!(
        !domain.files().is_empty(),
        "expected example_mod.srf to contain files"
    );

    for f in domain.files() {
        assert_eq!(f.file_md5().bytes().len(), 16, "file md5 must be 16 bytes");

        if let Some(parts) = f.parts() {
            let mut expected = 0u64;
            for p in parts {
                assert_eq!(p.md5.bytes().len(), 16, "part md5 must be 16 bytes");
                assert_eq!(
                    p.offset, expected,
                    "parts must be contiguous for {}",
                    f.rel_path().as_str()
                );
                expected = p.end_exclusive();
            }
            assert_eq!(
                expected,
                f.size(),
                "parts must cover full file for {}",
                f.rel_path().as_str()
            );
        }
    }
}

#[test]
fn parses_and_ingests_real_legacy_text_srf_fixture() {
    let bytes = read_fixture("legacy_text_mod.srf");
    assert_eq!(
        bytes.len(),
        30_793,
        "fixture size changed; update the asserted byte length if intentional"
    );

    let wire = parse_mod_srf(&bytes).expect("parse_mod_srf(legacy_text_mod.srf)");
    let domain = ingest_mod_srf(wire).expect("ingest_mod_srf(legacy_text_mod.srf)");

    let mod_id = domain.mod_id().as_str();
    assert!(!mod_id.trim().is_empty(), "expected non-empty mod id");
    assert!(mod_id.starts_with('@'), "expected '@' mod id, got {mod_id}");

    for f in domain.files() {
        assert_eq!(f.file_md5().bytes().len(), 16, "file md5 must be 16 bytes");
        if let Some(parts) = f.parts() {
            for p in parts {
                assert_eq!(p.md5.bytes().len(), 16, "part md5 must be 16 bytes");
            }
        }
    }
}

