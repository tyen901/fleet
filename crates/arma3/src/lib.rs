//! arma3_hemtt_wrapper
//!
//! Minimal wrapper around HEMTT's common utilities to:
//! - discover Arma 3 install from Steam
//! - build launch commands (native steam)
//! - validate local mod paths and build -mod= arguments
//! - spawn the game process

mod command;
mod error;
mod launcher;
mod mods;
mod steam;

pub use crate::command::{LaunchCommand, LaunchMethod};
pub use crate::error::{Error, Result};
pub use crate::launcher::{steam_available, Arma3Install, LaunchRequest, Launcher};
pub use crate::mods::{ModList, ModPathStyle};
pub use crate::steam::discover_steam_arma3;
