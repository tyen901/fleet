#[cfg(feature = "swifty")]
#[cfg(test)]
mod tests {
    use crate::errors::ManifestError;
    use crate::ingest::ingest_mod_manifest;
    use fleet_types::swifty::checksums::mod_checksum_from_files;
    use fleet_types::swifty::model as sw;
    use fleet_types::Md5Digest;
    use relative_path::RelativePathBuf;

    fn md5_from_u8(b: u8) -> Md5Digest {
        Md5Digest::from_bytes([b; 16])
    }

    #[test]
    fn ingest_valid_manifest() {
        let files = vec![sw::FileManifest {
            path: RelativePathBuf::from("foo/bar.pbo"),
            length: 18,
            checksum: md5_from_u8(0xBB),
            parts: vec![sw::PartManifest {
                start: 0,
                length: 18,
                checksum: md5_from_u8(0xCC),
            }],
        }];
        let manifest = sw::ModManifest {
            name: "mod1".to_string(),
            checksum: mod_checksum_from_files(&files),
            files,
        };
        let internal = ingest_mod_manifest(manifest).unwrap();
        assert_eq!(internal.mod_id().as_str(), "mod1");
        assert_eq!(internal.files().len(), 1);
        let file = &internal.files()[0];
        assert_eq!(file.rel_path().as_str(), "foo/bar.pbo");
        assert!(file.parts().is_some());
    }

    #[test]
    fn ingest_rejects_duplicate_files() {
        let files = vec![
            sw::FileManifest {
                path: RelativePathBuf::from("foo.pbo"),
                length: 1,
                checksum: md5_from_u8(0x05),
                parts: Vec::new(),
            },
            sw::FileManifest {
                path: RelativePathBuf::from("foo.pbo"),
                length: 1,
                checksum: md5_from_u8(0x05),
                parts: Vec::new(),
            },
        ];
        let manifest = sw::ModManifest {
            name: "mod2".to_string(),
            checksum: mod_checksum_from_files(&files),
            files,
        };
        let err = ingest_mod_manifest(manifest).unwrap_err();
        match err {
            ManifestError::DuplicateFile(path) => assert_eq!(path, "foo.pbo"),
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn ingest_rejects_invalid_parts() {
        let files = vec![sw::FileManifest {
            path: RelativePathBuf::from("data.bin"),
            length: 10,
            checksum: md5_from_u8(0x04),
            parts: vec![sw::PartManifest {
                start: 0,
                length: 5,
                checksum: md5_from_u8(0x05),
            }],
        }];
        let manifest = sw::ModManifest {
            name: "mod3".to_string(),
            checksum: mod_checksum_from_files(&files),
            files,
        };
        let err = ingest_mod_manifest(manifest).unwrap_err();
        match err {
            ManifestError::InvalidParts { rel_path, .. } => assert_eq!(rel_path, "data.bin"),
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn ingest_rejects_checksum_mismatch() {
        let files = vec![sw::FileManifest {
            path: RelativePathBuf::from("data.bin"),
            length: 10,
            checksum: md5_from_u8(0x04),
            parts: vec![sw::PartManifest {
                start: 0,
                length: 10,
                checksum: md5_from_u8(0x05),
            }],
        }];
        let manifest = sw::ModManifest {
            name: "mod4".to_string(),
            checksum: md5_from_u8(0xFF),
            files,
        };
        let err = ingest_mod_manifest(manifest).unwrap_err();
        match err {
            ManifestError::InvalidManifest(msg) => assert_eq!(msg, "mod checksum mismatch"),
            _ => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn ingest_rejects_part_offset_overflow() {
        let files = vec![sw::FileManifest {
            path: RelativePathBuf::from("data.bin"),
            length: u64::MAX,
            checksum: md5_from_u8(0x04),
            parts: vec![
                sw::PartManifest {
                    start: 0,
                    length: u64::MAX,
                    checksum: md5_from_u8(0x05),
                },
                sw::PartManifest {
                    start: u64::MAX,
                    length: 1,
                    checksum: md5_from_u8(0x06),
                },
            ],
        }];
        let manifest = sw::ModManifest {
            name: "mod5".to_string(),
            checksum: mod_checksum_from_files(&files),
            files,
        };
        let err = ingest_mod_manifest(manifest).unwrap_err();
        match err {
            ManifestError::InvalidParts { rel_path, msg } => {
                assert_eq!(rel_path, "data.bin");
                assert!(msg.contains("overflow"), "msg was: {msg}");
            }
            _ => panic!("unexpected error: {err:?}"),
        }
    }
}
