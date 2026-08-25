use dioxus::prelude::*;
use md_web_contracts::HealthSnapshot;
#[cfg(feature = "server")]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
mod config;
mod connections;
mod hive;
mod memory;
mod office;
mod pty;
mod team;
mod voice;
pub(crate) mod workspace;
pub(crate) use config::*;
pub(crate) use connections::*;
pub(crate) use hive::*;
#[cfg(feature = "server")]
pub(crate) use memory::knowledge_upload_multipart;
pub(crate) use memory::{
    activity_tail, base_skill_assignments, base_skills_catalog, base_skills_install, history_query,
    knowledge_get, knowledge_remove, knowledge_search, knowledge_upload, memory_graph, memory_mine,
    memory_reflect, memory_semantic_search, memory_skills_snapshot, memory_wake_up,
    save_base_skill_assignments, skills_catalog, skills_install, skills_local, skills_uninstall,
    telemetry_waterfall,
};
#[cfg(feature = "server")]
pub(crate) use memory::{
    cancel_memory_processes, record_activity_event, record_prompt_accepted,
    record_provider_transcript,
};
pub(crate) use office::*;
#[cfg(feature = "server")]
pub(crate) use pty::{agent_hook, provider_agent_hook, terminal_socket};
pub(crate) use pty::{
    list_agents, pty_input, pty_kill, pty_queue, pty_redraw, pty_resize, pty_restart, pty_restore,
    pty_spawn,
};
pub(crate) use team::onboarding_spawn_team;
#[cfg(feature = "server")]
pub(crate) use voice::voice_tls_paths;
pub(crate) use voice::{
    voice_action, voice_bootstrap, voice_cancel_action, voice_clear_provider_key,
    voice_confirm_action, voice_events, voice_mint_realtime_token, voice_record_realtime_usage,
    voice_set_freeflow_config, voice_set_realtime_cost_cap, voice_set_session_live,
    voice_transcribe, voice_write_provider_key,
};

#[cfg(feature = "server")]
static PERSISTENCE: tokio::sync::OnceCell<
    tokio::sync::Mutex<Option<md_web_services::domains::persistence::PgPersistenceRuntime>>,
> = tokio::sync::OnceCell::const_new();
#[cfg(feature = "server")]
static APP_STATE: tokio::sync::OnceCell<md_web_services::AppState> =
    tokio::sync::OnceCell::const_new();

#[cfg(feature = "server")]
pub(crate) async fn persistence_repository()
-> Result<md_web_services::domains::persistence::PgPersistenceRepository, ServerFnError> {
    let state = PERSISTENCE
        .get_or_init(|| async { tokio::sync::Mutex::new(None) })
        .await;
    let closing = CLOSING.load(Ordering::Acquire);
    let ready = open_slot_if_needed(state, closing, || async {
        md_web_services::domains::persistence::PgPersistenceRuntime::from_environment()
            .await
            .map_err(|_| ServerFnError::new("PostgreSQL永続化を利用できません"))
    })
    .await?;
    if !ready {
        return Err(ServerFnError::new("PostgreSQL永続化は終了処理中です"));
    }
    let guard = state.lock().await;
    let runtime = guard
        .as_ref()
        .ok_or_else(|| ServerFnError::new("PostgreSQL永続化を利用できません"))?;
    Ok(runtime.repository())
}

#[cfg(feature = "server")]
#[cfg(test)]
const fn persistence_open_allowed(has_runtime: bool, closing: bool) -> bool {
    !has_runtime && !closing
}

#[cfg(feature = "server")]
async fn open_slot_if_needed<T, E, Open, Future>(
    slot: &tokio::sync::Mutex<Option<T>>,
    closing: bool,
    open: Open,
) -> Result<bool, E>
where
    Open: FnOnce() -> Future,
    Future: std::future::Future<Output = Result<T, E>>,
{
    let mut guard = slot.lock().await;
    if guard.is_some() {
        return Ok(true);
    }
    if closing {
        return Ok(false);
    }
    *guard = Some(open().await?);
    Ok(true)
}

#[cfg(feature = "server")]
async fn close_persistence() {
    let Some(state) = PERSISTENCE.get() else {
        return;
    };
    if let Some(runtime) = state.lock().await.take() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), runtime.close()).await;
    }
}

#[cfg(feature = "server")]
static CLOSING: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "server")]
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "server")]
static IN_FLIGHT_CHANGED: tokio::sync::Notify = tokio::sync::Notify::const_new();
#[cfg(feature = "server")]
static SHUTDOWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(feature = "server")]
static CANCELLATION_CHANNEL: std::sync::OnceLock<tokio::sync::watch::Sender<bool>> =
    std::sync::OnceLock::new();
#[cfg(feature = "server")]
static TERMINATION_CHANNEL: std::sync::OnceLock<tokio::sync::watch::Sender<bool>> =
    std::sync::OnceLock::new();
#[cfg(feature = "server")]
static STREAM_RESET_CHANNEL: std::sync::OnceLock<tokio::sync::watch::Sender<u64>> =
    std::sync::OnceLock::new();

#[cfg(feature = "server")]
fn cancellation_sender() -> &'static tokio::sync::watch::Sender<bool> {
    CANCELLATION_CHANNEL.get_or_init(|| tokio::sync::watch::channel(false).0)
}

#[cfg(feature = "server")]
fn termination_sender() -> &'static tokio::sync::watch::Sender<bool> {
    TERMINATION_CHANNEL.get_or_init(|| tokio::sync::watch::channel(false).0)
}

#[cfg(feature = "server")]
pub(crate) fn shutdown_receiver() -> tokio::sync::watch::Receiver<bool> {
    cancellation_sender().subscribe()
}

#[cfg(feature = "server")]
fn stream_reset_sender() -> &'static tokio::sync::watch::Sender<u64> {
    STREAM_RESET_CHANNEL.get_or_init(|| tokio::sync::watch::channel(0).0)
}

#[cfg(feature = "server")]
pub(crate) fn stream_reset_receiver() -> tokio::sync::watch::Receiver<u64> {
    stream_reset_sender().subscribe()
}

#[cfg(feature = "server")]
pub(crate) async fn wait_for_shutdown() {
    let mut receiver = termination_sender().subscribe();
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}

#[cfg(feature = "server")]
pub(crate) async fn shutdown_application(from_request: bool) -> Result<(), ()> {
    let _shutdown_guard = SHUTDOWN_LOCK.lock().await;
    if *termination_sender().borrow() {
        return Ok(());
    }

    CLOSING.store(true, Ordering::Release);
    let _ = cancellation_sender().send(true);
    cancel_memory_processes();
    let allowed = usize::from(from_request);
    let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if drain_ready(IN_FLIGHT.load(Ordering::Acquire), allowed) {
                break;
            }
            IN_FLIGHT_CHANGED.notified().await;
        }
    })
    .await
    .is_ok();
    if !drained {
        if from_request {
            CLOSING.store(false, Ordering::Release);
            let _ = cancellation_sender().send(false);
        } else {
            close_persistence().await;
            eprintln!("GRACEFUL_SHUTDOWN_FAILED stage=drain");
            let _ = termination_sender().send(true);
        }
        return Err(());
    }

    let connections_stopped = connections_stop_slack().await.is_ok()
        & connections_stop_webhooks().await.is_ok()
        & connections_stop_broker().await.is_ok()
        & shutdown_provider_auth().is_ok();
    let ptys_stopped = pty::shutdown_all().await.is_ok();
    if !(connections_stopped && ptys_stopped) {
        if from_request {
            CLOSING.store(false, Ordering::Release);
            let _ = cancellation_sender().send(false);
        } else {
            close_persistence().await;
            eprintln!("GRACEFUL_SHUTDOWN_FAILED stage=producers");
            let _ = termination_sender().send(true);
        }
        return Err(());
    }

    close_persistence().await;
    eprintln!("GRACEFUL_SHUTDOWN_OK");
    let _ = termination_sender().send(true);
    Ok(())
}

#[cfg(feature = "server")]
pub(crate) struct NamespaceResetGuard {
    _shutdown_guard: tokio::sync::MutexGuard<'static, ()>,
}

#[cfg(feature = "server")]
impl Drop for NamespaceResetGuard {
    fn drop(&mut self) {
        CLOSING.store(false, Ordering::Release);
    }
}

#[cfg(feature = "server")]
pub(crate) async fn begin_namespace_reset() -> Result<NamespaceResetGuard, ()> {
    let shutdown_guard = SHUTDOWN_LOCK.lock().await;
    CLOSING.store(true, Ordering::Release);
    let next_epoch = stream_reset_sender().borrow().wrapping_add(1);
    let _ = stream_reset_sender().send(next_epoch);
    let drained = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if drain_ready(IN_FLIGHT.load(Ordering::Acquire), 1) {
                break;
            }
            IN_FLIGHT_CHANGED.notified().await;
        }
    })
    .await
    .is_ok();
    if !drained {
        CLOSING.store(false, Ordering::Release);
        return Err(());
    }
    Ok(NamespaceResetGuard {
        _shutdown_guard: shutdown_guard,
    })
}

#[cfg(feature = "server")]
const fn drain_ready(in_flight: usize, allowed_current: usize) -> bool {
    in_flight <= allowed_current
}

#[cfg(feature = "server")]
struct InFlightGuard;

#[cfg(feature = "server")]
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
        IN_FLIGHT_CHANGED.notify_waiters();
    }
}

#[cfg(feature = "server")]
pub(crate) async fn admission_middleware(
    request: dioxus::server::axum::extract::Request,
    next: dioxus::server::axum::middleware::Next,
) -> dioxus::server::axum::response::Response {
    use dioxus::server::axum::response::IntoResponse;
    if CLOSING.load(Ordering::Acquire) {
        return (
            dioxus::server::axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "終了処理中です",
        )
            .into_response();
    }
    IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
    let _guard = InFlightGuard;
    next.run(request).await
}

#[get("/api/health")]
pub(crate) async fn health_status() -> Result<HealthSnapshot, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let state = APP_STATE
            .get_or_init(|| md_web_services::AppState::initialize(env!("CARGO_PKG_VERSION")))
            .await;
        // A health refresh is also the bounded recovery edge for a database that
        // was unavailable during the first request. Failed opens are never cached.
        let _ = persistence_repository().await;
        Ok(state.refresh_health_snapshot().await)
    }

    #[cfg(not(feature = "server"))]
    {
        Err(ServerFnError::new("health service is server-only"))
    }
}

#[cfg(all(test, feature = "server"))]
mod lifecycle_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{drain_ready, open_slot_if_needed, persistence_open_allowed};

    #[test]
    fn explicit_shutdown_allows_only_its_current_request() {
        assert!(drain_ready(1, 1));
        assert!(!drain_ready(2, 1));
    }

    #[test]
    fn signal_shutdown_waits_for_every_request() {
        assert!(drain_ready(0, 0));
        assert!(!drain_ready(1, 0));
    }

    #[test]
    fn failed_persistence_open_remains_retryable_until_success_or_shutdown() {
        assert!(persistence_open_allowed(false, false));
        assert!(persistence_open_allowed(false, false));
        assert!(!persistence_open_allowed(true, false));
        assert!(!persistence_open_allowed(false, true));
    }

    #[tokio::test]
    async fn persistence_slot_recovers_once_and_never_reopens_during_shutdown() {
        let slot = tokio::sync::Mutex::new(None::<&'static str>);
        let attempts = AtomicUsize::new(0);
        let first = open_slot_if_needed(&slot, false, || async {
            attempts.fetch_add(1, Ordering::Relaxed);
            Err::<&'static str, &'static str>("unreachable")
        })
        .await;
        assert_eq!(first, Err("unreachable"));
        assert!(slot.lock().await.is_none());

        let second = open_slot_if_needed(&slot, false, || async {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok::<&'static str, &'static str>("ready")
        })
        .await;
        assert_eq!(second, Ok(true));
        assert_eq!(*slot.lock().await, Some("ready"));

        let closed_slot = tokio::sync::Mutex::new(None::<&'static str>);
        let closed = open_slot_if_needed(&closed_slot, true, || async {
            attempts.fetch_add(1, Ordering::Relaxed);
            Ok::<&'static str, &'static str>("must-not-open")
        })
        .await;
        assert_eq!(closed, Ok(false));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }
}
