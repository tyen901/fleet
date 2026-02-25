use dioxus::prelude::*;
use dioxus_router::Routable;

use crate::app::shell::ShellLayout;
use crate::features::{
    boot::Boot,
    home::Home,
    not_found::PageNotFound,
    onboarding::Onboarding,
    profiles::{new::NewProfile, ProfileEdit, ProfileView},
    settings::Settings,
};

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[route("/")]
    Boot {},

    #[layout(ShellLayout)]
        #[route("/home")]
        Home {},
        #[route("/profiles/new")]
        NewProfile {},
        #[route("/profiles/:id")]
        ProfileView { id: String },
        #[route("/profiles/:id/edit")]
        ProfileEdit { id: String },
        #[route("/settings")]
        Settings {},
    #[end_layout]

    #[route("/onboarding")]
    Onboarding {},

    #[route("/:..route")]
    PageNotFound { route: Vec<String> },
}
