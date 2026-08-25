#![forbid(unsafe_code)]
#![deny(clippy::expect_used, clippy::unwrap_used)]

mod app;
mod components;
mod routes;
mod server_fns;

#[cfg(not(feature = "server"))]
fn main() {
    dioxus::launch(app::App);
}

#[cfg(feature = "server")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use dioxus::server::axum::{
        Router, middleware,
        routing::{get, post},
    };
    use dioxus::server::{DioxusRouterExt, ServeConfig};

    let ip = std::env::var("IP").unwrap_or_else(|_| String::from("127.0.0.1"));
    let port = std::env::var("PORT").unwrap_or_else(|_| String::from("5080"));
    let address: std::net::SocketAddr = format!("{ip}:{port}").parse()?;
    let tls = server_fns::voice_tls_paths()?;
    let router = Router::new()
        .route("/ws/terminal", get(server_fns::terminal_socket))
        .route("/internal/hive-hook", post(server_fns::agent_hook))
        .route(
            "/internal/hive-hook/{provider}",
            post(server_fns::provider_agent_hook),
        )
        .route(
            "/api/hive/events/stream",
            get(server_fns::hive_event_stream),
        )
        .route(
            "/api/memory-skills/knowledge/upload-multipart",
            post(server_fns::knowledge_upload_multipart),
        )
        .serve_dioxus_application(ServeConfig::new(), app::App)
        .layer(middleware::from_fn(server_fns::admission_middleware));
    if let Some(tls) = tls {
        rustls::crypto::aws_lc_rs::default_provider()
            .install_default()
            .map_err(|_| "TLS crypto provider is already configured incompatibly")?;
        let config =
            axum_server::tls_rustls::RustlsConfig::from_pem_file(tls.cert_path, tls.key_path)
                .await?;
        let handle = axum_server::Handle::new();
        let shutdown_handle = handle.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            shutdown_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });
        axum_server::bind_rustls(address, config)
            .handle(handle)
            .serve(router.into_make_service())
            .await?;
    } else {
        let listener = tokio::net::TcpListener::bind(address).await?;
        dioxus::server::axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await?;
    }
    Ok(())
}

#[cfg(feature = "server")]
async fn shutdown_signal() {
    tokio::select! {
        _ = external_shutdown_signal() => {
            let _ = server_fns::shutdown_application(false).await;
        }
        _ = server_fns::wait_for_shutdown() => {}
    }
}

#[cfg(feature = "server")]
async fn external_shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = terminate.recv() => {},
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
