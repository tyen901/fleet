pub mod error;
pub mod runner;

pub use error::PlatformError;
pub use runner::{execute, LaunchAction};

pub use crate::settings::OpenMode;
