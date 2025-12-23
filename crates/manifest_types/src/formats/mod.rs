pub mod legacy_srf;
pub mod mod_manifest;
pub mod pbo;
pub mod repo_json;

pub fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    const BOM: &[u8] = b"\xEF\xBB\xBF";
    if bytes.starts_with(BOM) {
        &bytes[BOM.len()..]
    } else {
        bytes
    }
}

pub fn is_legacy_srf(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    trimmed.starts_with("ADDON")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_utf8_bom_removes_bom() {
        let with = b"\xEF\xBB\xBFhello";
        let without = strip_utf8_bom(with);
        assert_eq!(without, b"hello");
    }

    #[test]
    fn strip_utf8_bom_noop_when_missing() {
        let b = b"hello";
        assert_eq!(strip_utf8_bom(b), b);
    }

    #[test]
    fn is_legacy_srf_handles_bom_and_whitespace() {
        let b = b"\xEF\xBB\xBF  \nADDON:foo:0:0123456789ABCDEF0123456789ABCDEF\n";
        assert!(is_legacy_srf(b));
    }

    #[test]
    fn is_legacy_srf_false_for_json() {
        let b = br#"{"name":"x","checksum":"0123456789ABCDEF0123456789ABCDEF","files":[]}"#;
        assert!(!is_legacy_srf(b));
    }
}
