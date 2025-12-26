use crate::ports::Checksummer;
use anyhow::Result;
use fleet_manifest::ManifestPart;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct VerifyMismatch {
    pub(crate) offset: u64,
    pub(crate) len: u64,
}

pub(crate) fn first_mismatch(
    path: &std::path::Path,
    expected_size: u64,
    expected_file_md5: &[u8; 16],
    parts: Option<&[ManifestPart]>,
    checksummer: &dyn Checksummer,
) -> Result<Option<VerifyMismatch>> {
    match parts {
        None | Some([]) => {
            let got = crate::md5::vec_to_md5_16(checksummer.hash_file(path)?)?;
            if &got != expected_file_md5 {
                return Ok(Some(VerifyMismatch {
                    offset: 0,
                    len: expected_size,
                }));
            }
            Ok(None)
        }
        Some(parts) => crate::verify_parts::first_part_mismatch(path, parts, checksummer).map(
            |opt| opt.map(|(offset, len)| VerifyMismatch { offset, len }),
        ),
    }
}

