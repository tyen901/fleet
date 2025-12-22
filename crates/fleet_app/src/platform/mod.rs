pub mod error;
pub mod runner;

pub use error::PlatformError;
pub use runner::{execute, open_path, LaunchAction};

pub use crate::settings::OpenMode;
