use dioxus::prelude::*;

use crate::components::dashboard::HealthViewState;
use crate::components::shell::AppShell;
use crate::server_fns::health_status;

const APP_CSS: Asset = asset!("/assets/app.css");

#[component]
pub(crate) fn App() -> Element {
    let mut health = use_resource(health_status);
    let health_view = match health.read().as_ref() {
        None => HealthViewState::Loading,
        Some(Ok(snapshot)) => HealthViewState::Ready(snapshot.clone()),
        Some(Err(error)) => HealthViewState::Error(error.to_string()),
    };

    rsx! {
        document::Title { "Munder Difflin" }
        document::Link { rel: "stylesheet", href: APP_CSS }
        AppShell {
            health: health_view,
            on_refresh: move |_| health.restart(),
        }
    }
}
