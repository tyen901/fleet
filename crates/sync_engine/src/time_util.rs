pub fn now_ns() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

pub fn file_mtime_ns(md: &std::fs::Metadata) -> Option<crate::model::TimestampNs> {
    let nanos = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    i64::try_from(nanos).ok().map(crate::model::TimestampNs)
}
