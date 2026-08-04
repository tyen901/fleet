use dioxus::prelude::*;
use dioxus_router::Routable;

use crate::app::shell::ShellLayout;
use crate::features::{
    boot::Boot,
    not_found::PageNotFound,
    onboarding::Onboarding,
    profiles::{list::Profiles, new::NewProfile, view::ProfileView},
    settings::Settings,
};

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    Boot {},

    #[layout(ShellLayout)]
        #[route("/profiles")]
        Profiles {},
        #[route("/profiles/new")]
        NewProfile {},
        #[route("/profiles/:id")]
        ProfileView { id: String },
        #[route("/settings")]
        Settings {},
    #[end_layout]

    #[route("/onboarding")]
    Onboarding {},

    #[route("/:..route")]
    PageNotFound { route: Vec<String> },
}
