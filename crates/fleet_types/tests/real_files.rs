use std::fs;
use std::io::{BufReader, Seek};
use std::path::{Path, PathBuf};

use fleet_types::arma::pbo::{partition_pbo, read_pbo_meta};
use fleet_types::{
    file_checksum_from_parts, mod_checksum_from_files, validate_parts, ModManifest, RepoSpec,
};

fn test_files_root() -> PathBuf {
    // crates/fleet_types -> crates -> repo root -> test_files
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test_files")
}

fn read_bytes(rel: &str) -> Vec<u8> {
    let p = test_files_root().join(rel);
    fs::read(&p).unwrap_or_else(|e| panic!("failed to read {}: {e}", p.display()))
}

fn assert_manifest_invariants(m: &ModManifest) {
    assert!(
        !m.name.trim().is_empty(),
        "manifest name should not be empty"
    );
    assert!(!m.files.is_empty(), "manifest should contain files");

    // Confirm paths are normalized, and list is sorted (parse_any sorts).
    let mut last = "";
    for f in &m.files {
        let s = f.path.as_str();
        assert!(
            !s.contains('\\'),
            "paths must be normalized to forward slashes, got: {s}"
        );
        assert!(s >= last, "files must be sorted by path; {s} < {last}");
        last = s;

        // Parts should validate and cover file length.
        let sorted = validate_parts(&f.parts, f.length)
            .unwrap_or_else(|e| panic!("parts failed validation for {}: {e}", s));
        assert_eq!(
            sorted.len(),
            f.parts.len(),
            "validate_parts should not drop parts"
        );

        // Checksum correctness: file checksum should match checksum derived from its parts.
        let derived_file = file_checksum_from_parts(&f.parts);
        assert_eq!(derived_file, f.checksum, "file checksum mismatch for {s}");
    }

    // Checksum correctness: mod checksum should match checksum derived from its files.
    let derived_mod = mod_checksum_from_files(&m.files);
    assert_eq!(
        derived_mod, m.checksum,
        "mod checksum mismatch for {}",
        m.name
    );
}

#[test]
fn parses_real_legacy_srf_root_mod_srf() {
    let bytes = read_bytes("mod.srf");
    let m = ModManifest::from_bytes(&bytes).expect("parse root test_files/mod.srf");
    assert_manifest_invariants(&m);
}

#[test]
fn parses_real_legacy_srf_ace_compat_mod_srf() {
    let bytes = read_bytes("@ace_compat_cup_vehicles/mod.srf");
    let m = ModManifest::from_bytes(&bytes).expect("parse @ace_compat_cup_vehicles/mod.srf");
    assert_manifest_invariants(&m);
}

#[test]
fn parses_real_repo_json() {
    let bytes = read_bytes("repo.json");
    let repo = RepoSpec::from_bytes(&bytes).expect("parse test_files/repo.json");

    assert!(
        !repo.repo_name.trim().is_empty(),
        "repo_name should not be empty"
    );
    assert!(
        !repo.version.trim().is_empty(),
        "version should not be empty"
    );
    assert!(
        !repo.client_parameters.trim().is_empty(),
        "client_parameters should not be empty"
    );

    // Confirm at least one mod exists, and checksums are decodable MD5 digests (Md5Digest already parsed).
    assert!(
        !repo.required_mods.is_empty() || !repo.optional_mods.is_empty(),
        "repo should have at least one mod listed"
    );

    for m in repo.required_mods.iter().chain(repo.optional_mods.iter()) {
        // Md5Digest is a fixed 16 bytes; to_hex_upper is a 32-char string.
        assert_eq!(
            m.checksum.to_hex_upper().len(),
            32,
            "checksum should be 32 hex chars"
        );
    }

    // Servers may be empty; if present, confirm ports are within u16 (already enforced by serde).
    for s in &repo.servers {
        assert!(s.port > 0, "server port should be non-zero for {}", s.name);
    }
}

#[test]
fn partitions_real_pbos() {
    // Use the real PBOs in test_files.
    let pbo_paths = [
        "@ace/addons/ace_advanced_ballistics.pbo",
        "@ace_compat_cup_vehicles/addons/cup_vehicles_ace_compat.pbo",
    ];

    for rel in pbo_paths {
        let p = test_files_root().join(rel);
        let f = fs::File::open(&p).unwrap_or_else(|e| panic!("open {}: {e}", p.display()));
        let len = f.metadata().unwrap().len();

        let mut r = BufReader::new(f);

        // Read meta first; this also implicitly tests read_pbo_meta on real data.
        let meta = read_pbo_meta(&mut r).unwrap_or_else(|e| panic!("read_pbo_meta {}: {e}", rel));
        assert!(
            meta.header_len <= len,
            "header_len must not exceed file length"
        );
        assert!(
            !meta.entries.is_empty(),
            "pbo should have at least one entry"
        );
        assert_eq!(
            meta.entries[0].data_size, 0,
            "pbo first entry must have data_size == 0 (Swifty/Nimble compatibility rule)"
        );

        // Now partition and confirm coverage and monotonicity.
        r.rewind().expect("rewind reader");
        let parts = partition_pbo(&mut r, len).unwrap_or_else(|e| panic!("partition {}: {e}", rel));

        assert!(!parts.is_empty(), "parts should not be empty");
        assert_eq!(parts[0].0, 0, "first part must start at 0");
        assert_eq!(
            parts[0].1, meta.header_len,
            "first part length must equal header_len"
        );

        let mut total = 0u64;
        let mut last_end = 0u64;

        for (start, plen) in &parts {
            assert_eq!(*start, last_end, "parts must be contiguous");
            total = total.saturating_add(*plen);
            last_end = start.saturating_add(*plen);
        }

        assert_eq!(total, len, "parts must cover file length exactly");
        assert_eq!(last_end, len, "last part must end at file length");
    }
}
