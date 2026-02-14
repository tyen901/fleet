use inventory::{open_flux_inventory, FinalizedFileRecord, SegmentSignature};
use std::path::PathBuf;

#[test]
fn flux_inventory_api_round_trips_finalized_records_and_segment_queries() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("inv.db");
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).expect("create root");

    let inv = open_flux_inventory(&db_path, "inv", &root).expect("open api");

    let sig_a = SegmentSignature {
        scheme: "md5".to_string(),
        value_hex: "aaaaaaaa".to_string(),
        size_bytes: 4,
    };
    let sig_b = SegmentSignature {
        scheme: "md5".to_string(),
        value_hex: "bbbbbbbb".to_string(),
        size_bytes: 4,
    };

    inv.record_finalized_file_batch(&[
        FinalizedFileRecord {
            rel_path: PathBuf::from("a.bin"),
            size_bytes: 8,
            mtime_ns: 1,
            segments: vec![(sig_a.clone(), 4), (sig_b.clone(), 4)],
        },
        FinalizedFileRecord {
            rel_path: PathBuf::from("b.bin"),
            size_bytes: 4,
            mtime_ns: 1,
            segments: vec![(sig_a.clone(), 4)],
        },
    ])
    .expect("record batch");

    let out = inv
        .get_segment_locations_batch(&[sig_a.clone(), sig_b.clone()])
        .expect("segment locations batch");

    assert_eq!(out.len(), 2);
    assert_eq!(out[0].len(), 2);
    assert_eq!(out[1].len(), 1);
    assert!(inv
        .has_segment_location(PathBuf::from("a.bin").as_path(), &sig_b, 4, 4)
        .expect("has segment"));
}
