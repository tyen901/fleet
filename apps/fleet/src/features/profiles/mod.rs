pub const PROFILE_NAME_PLACEHOLDER: &str = "My Profile";
pub const PROFILE_REPO_URL_PLACEHOLDER: &str = "https://example.com/repo.json";
pub const PROFILE_TARGET_FOLDER_PLACEHOLDER: &str = "/path/to/arma3";

pub(crate) mod common;
pub mod draft;
pub mod edit;
pub mod new;
pub mod view;

pub use edit::ProfileEdit;
pub use view::ProfileView;
