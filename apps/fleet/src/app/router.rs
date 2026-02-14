use dioxus::prelude::*;
use dioxus_router::Routable;

use crate::app::shell::MainLayout;
use crate::features::{
    dashboard::Dashboard,
    not_found::PageNotFound,
    onboarding::Onboarding,
    profiles::{edit::EditProfile, new::NewProfile},
    settings::Settings,
};

#[derive(Clone, Routable, Debug, PartialEq)]
#[rustfmt::skip]
pub enum Route {
    #[layout(MainLayout)]
        #[route("/")]
        Dashboard {},
        #[route("/new")]
        NewProfile {},
        #[route("/edit/:id")]
        EditProfile { id: String },
        #[route("/settings")]
        Settings {},
    #[end_layout]

    #[route("/onboarding")]
    Onboarding {},

    #[route("/:..route")]
    PageNotFound { route: Vec<String> },
}
