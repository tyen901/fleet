pub const CACHE_KEY_SEPARATOR: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheKey<'a> {
    pub mod_name: &'a str,
    pub rel_path: &'a str,
}

impl<'a> CacheKey<'a> {
    pub fn new(mod_name: &'a str, rel_path: &'a str) -> Self {
        Self { mod_name, rel_path }
    }

    pub fn prefix_for_mod(mod_name: &str) -> Vec<u8> {
        let mut prefix = Vec::with_capacity(mod_name.len() + 1);
        prefix.extend_from_slice(mod_name.as_bytes());
        prefix.push(CACHE_KEY_SEPARATOR);
        prefix
    }

    pub fn to_bytes(self) -> Vec<u8> {
        let mut key = Vec::with_capacity(self.mod_name.len() + 1 + self.rel_path.len());
        key.extend_from_slice(self.mod_name.as_bytes());
        key.push(CACHE_KEY_SEPARATOR);
        key.extend_from_slice(self.rel_path.as_bytes());
        key
    }

    pub fn rel_path_from_prefixed_key<'k>(prefix: &[u8], full_key: &'k [u8]) -> Option<&'k str> {
        let rel_bytes = full_key.strip_prefix(prefix)?;
        std::str::from_utf8(rel_bytes).ok()
    }
}
