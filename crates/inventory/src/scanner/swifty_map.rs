use crate::scanner::walk::WalkItem;
use crate::{FileEntry, SegmentEntry};
use swifty_artifacts::{SrfFile, SrfPart};

pub fn scan_one(
    item: &WalkItem,
) -> Result<(FileEntry, Vec<SegmentEntry>), swifty_artifacts::SwiftyError> {
    let srf: SrfFile = swifty_artifacts::scan_file(&item.fs_path, &item.rel_path)?;

    let checksum = Some(srf.checksum.to_hex_upper());

    let file = FileEntry {
        rel_path: fleet_domain::normalize_rel_slashes(&item.rel_path),
        length: srf.length,
        checksum,
    };

    let segs = map_parts(&srf.parts);
    Ok((file, segs))
}

fn map_parts(parts: &[SrfPart]) -> Vec<SegmentEntry> {
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| SegmentEntry {
            idx: i as u32,
            name: p.path.clone(),
            start: p.start,
            length: p.length,
            checksum: p.checksum.to_hex_upper(),
        })
        .collect()
}
