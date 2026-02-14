pub fn now_unix_ms() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be >= unix epoch");
    now.as_millis() as u64
}
