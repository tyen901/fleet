pub(crate) fn hash_file_record(rel: &str, len: u64) -> u64 {
    const OFFSET: u64 = 14695981039346656037;
    const PRIME: u64 = 1099511628211;

    let mut h = OFFSET;
    for &b in rel.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h ^= 0xFF;
    h = h.wrapping_mul(PRIME);

    for &b in len.to_le_bytes().as_slice() {
        h ^= b as u64;
        h = h.wrapping_mul(PRIME);
    }
    h
}

pub(crate) fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::{hash_file_record, mix64};

    #[test]
    fn hash_file_record_matches_known_vectors() {
        assert_eq!(hash_file_record("addons/a.pbo", 123), 16248981927443790643);
        assert_eq!(hash_file_record("x", 0), 10740132563774347112);
        assert_eq!(
            hash_file_record("dir/sub/file.bin", 9_876_543_210),
            989910769967872274
        );
    }

    #[test]
    fn mix64_matches_known_vectors() {
        assert_eq!(mix64(16248981927443790643), 8034750166916460283);
        assert_eq!(mix64(10740132563774347112), 14616538206972795866);
        assert_eq!(mix64(989910769967872274), 9652661096119338810);
    }
}
