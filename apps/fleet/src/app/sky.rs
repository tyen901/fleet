use dioxus::desktop::{use_asset_handler, wry::http::Response};
use dioxus::prelude::*;
use fleet_domain::SkyLocation;

#[component]
pub(crate) fn SkyBackground(location: Option<SkyLocation>) -> Element {
    let (latitude, longitude) = location.unwrap_or(SkyLocation::WELLINGTON).degrees();
    use_asset_handler("sky", |request, responder| {
        let script = "text/javascript; charset=utf-8";
        let asset: Option<(&str, &[u8])> = match request.uri().path() {
            "/sky/sky-background.mjs" => Some((
                script,
                include_bytes!("../../assets/sky/sky-background.mjs"),
            )),
            "/sky/sky-chart.mjs" => {
                Some((script, include_bytes!("../../assets/sky/sky-chart.mjs")))
            }
            "/sky/sky-model.mjs" => {
                Some((script, include_bytes!("../../assets/sky/sky-model.mjs")))
            }
            "/sky/satellite-engine.mjs" => Some((
                script,
                include_bytes!("../../assets/sky/satellite-engine.mjs"),
            )),
            "/sky/satellite-tracks.mjs" => Some((
                script,
                include_bytes!("../../assets/sky/satellite-tracks.mjs"),
            )),
            "/sky/satellites.json" => Some((
                "application/json",
                include_bytes!("../../assets/sky/satellites.json"),
            )),
            "/sky/astronomy-engine.mjs" => Some((
                script,
                include_bytes!("../../assets/sky/astronomy-engine.mjs"),
            )),
            "/sky/stars.json" => Some((
                "application/json",
                include_bytes!("../../assets/sky/stars.json"),
            )),
            _ => None,
        };
        let response = match asset {
            Some((content_type, bytes)) => Response::builder()
                .header("Content-Type", content_type)
                .body(bytes.to_vec()),
            None => Response::builder().status(404).body(Vec::new()),
        };
        responder.respond(response.expect("static sky asset response"));
    });

    rsx! {
        document::Script { src: "/sky/sky-background.mjs", r#type: "module" }
        wellington-sky {
            class: "profiles-page__sky",
            "aria-hidden": "true",
            "location": "{latitude},{longitude}",
        }
    }
}
