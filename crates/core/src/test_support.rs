#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
pub(crate) static ENV_VAR_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub(crate) struct EnvVarGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl EnvVarGuard {
    pub(crate) fn set_path(key: &'static str, value: &std::path::Path) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, old }
    }
}

#[cfg(test)]
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.old.take() {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}
