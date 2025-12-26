use std::path::PathBuf;

use fleet_arma_wire::{partition_pbo, read_pbo_meta};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

#[test]
fn can_read_and_partition_real_fixture_pbo() {
    let path = fixture_root().join("example_pbo.pbo");
    let f = std::fs::File::open(&path).expect("open fixture pbo");
    let file_len = f.metadata().expect("stat fixture pbo").len();
    assert!(file_len > 0, "fixture pbo should not be empty");

    let mut reader = std::io::BufReader::new(f);

    let meta = read_pbo_meta(&mut reader).expect("read_pbo_meta");
    assert!(meta.header_len > 0, "expected non-zero header length");
    assert!(
        meta.header_len <= file_len,
        "header_len should be <= file_len"
    );
    assert!(!meta.entries.is_empty(), "expected at least one pbo entry");

    let parts = partition_pbo(&mut reader, file_len).expect("partition_pbo");
    assert!(!parts.is_empty(), "expected non-empty partitions");

    let mut offset = 0u64;
    for (start, len) in &parts {
        assert_eq!(*start, offset, "expected contiguous partitioning");
        assert!(*len > 0, "expected non-zero part length");
        offset = offset.saturating_add(*len);
    }
    assert_eq!(offset, file_len, "expected partitions cover full file");
}
