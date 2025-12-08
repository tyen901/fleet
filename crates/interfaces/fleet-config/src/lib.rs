//! Central configuration constants for runtime limits and defaults.

/// Default number of concurrent download threads.
pub const DEFAULT_DOWNLOAD_THREADS: usize = 4;

/// Minimum allowed concurrent download threads.
pub const MIN_DOWNLOAD_THREADS: usize = 1;

/// Maximum allowed concurrent download threads.
pub const MAX_DOWNLOAD_THREADS: usize = 8;

/// Default speed limit when enabled (bytes per second). 5 MB/s.
pub const DEFAULT_SPEED_LIMIT_BYTES: u64 = 5 * 1024 * 1024;

/// Convenience function to clamp a thread value into allowed range.
pub fn clamp_threads(v: usize) -> usize {
    v.clamp(MIN_DOWNLOAD_THREADS, MAX_DOWNLOAD_THREADS)
}
