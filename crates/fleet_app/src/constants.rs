/// Shared constants that must not introduce dependency edges between modules.
/// (e.g., avoid `launch` depending on `registry` just to access a default string.)
pub const ARMA3_DEFAULT_EXTRA_ARGS: &str = "-noPause -noSplash -skipIntro -noLauncher";
