use dioxus::prelude::*;
use md_web_contracts::HealthSnapshot;

#[get("/api/health")]
pub(crate) async fn health_status() -> Result<HealthSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let state = md_web_services::AppState::initialize(env!("CARGO_PKG_VERSION")).await;
        Ok(state.health_snapshot())
    }

    #[cfg(not(feature = "server"))]
    {
        Err(ServerFnError::new("health service is server-only"))
    }
}
