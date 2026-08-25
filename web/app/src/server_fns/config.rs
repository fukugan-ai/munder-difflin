use dioxus::prelude::*;
use md_web_contracts::domains::config_onboarding::{
    AppInfo, CapabilitySupport, ChangeHomeRequest, ConfigPatch, ConfigRuntimeReceipt,
    ConfirmTeamInitializedRequest, ConfirmTeamInitializedResult, CreateFloorRequest,
    CreateFloorResponse, FinishOnboardingRequest, FinishOnboardingResult,
    OnboardingPathProbeRequest, ProviderKeyWrite, PublicConfig, ReleaseRepository,
    ResetNamespaceRequest, ResetResult, SecretPresence, SetAgentTokenCapRequest, ShutdownRequest,
    ShutdownResult, ToolStatus, UpdateStatus, ValidatedOnboardingPaths,
};
#[cfg(feature = "server")]
use md_web_contracts::domains::connections::WriteOnlySecret;
#[cfg(feature = "server")]
use md_web_services::domains::connections::{ProviderSecretId, SecretId, SecretProvider};

#[cfg(feature = "server")]
fn provider_secret_id(provider: &str) -> Result<ProviderSecretId, ServerFnError> {
    match provider {
        "openai" => Ok(ProviderSecretId::OpenAi),
        "groq" => Ok(ProviderSecretId::Groq),
        _ => Err(ServerFnError::new(
            "このプロバイダーのキー保存にはまだ対応していません",
        )),
    }
}

#[cfg(feature = "server")]
async fn provider_key_presence() -> Result<SecretPresence, ServerFnError> {
    let provider = super::connections::hydrated_secret_provider().await?;
    let openai_api_key = provider
        .has_secret(&SecretId::Provider(ProviderSecretId::OpenAi))
        .map_err(|_| safe_error())?;
    let groq_api_key = provider
        .has_secret(&SecretId::Provider(ProviderSecretId::Groq))
        .map_err(|_| safe_error())?;
    let mut provider_keys = Vec::new();
    if openai_api_key {
        provider_keys.push(String::from("openai"));
    }
    if groq_api_key {
        provider_keys.push(String::from("groq"));
    }
    Ok(SecretPresence {
        groq_api_key,
        openai_api_key,
        provider_keys,
        ..SecretPresence::default()
    })
}

#[cfg(feature = "server")]
async fn overlay_provider_key_presence(config: &mut PublicConfig) -> Result<(), ServerFnError> {
    let presence = provider_key_presence().await?;
    config.secrets.groq_api_key = presence.groq_api_key;
    config.secrets.openai_api_key = presence.openai_api_key;
    config.secrets.provider_keys = presence.provider_keys;
    Ok(())
}

#[cfg(feature = "server")]
fn safe_error() -> ServerFnError {
    ServerFnError::new("設定サービスの操作に失敗しました")
}

#[cfg(feature = "server")]
async fn repository()
-> Result<md_web_services::domains::persistence::PgPersistenceRepository, ServerFnError> {
    super::persistence_repository()
        .await
        .map_err(|_| safe_error())
}

#[cfg(feature = "server")]
fn release_repository() -> Result<ReleaseRepository, ServerFnError> {
    let configured = std::env::var("MD_RELEASE_REPO").ok();
    md_web_services::domains::config_onboarding::resolve_release_repository(configured.as_deref())
        .map_err(|_| ServerFnError::new("forkの更新元を確認できません"))
}

#[cfg(feature = "server")]
fn server_home() -> Result<std::path::PathBuf, ServerFnError> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| ServerFnError::new("サーバーのHOMEを確認できません"))
}

#[cfg(feature = "server")]
fn onboarding_roots(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    let configured = std::env::var_os("MD_ONBOARDING_ROOTS")
        .map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .unwrap_or_default();
    if configured.is_empty() {
        vec![home.to_path_buf()]
    } else {
        configured
    }
}

#[cfg(feature = "server")]
fn validate_paths(
    request: &OnboardingPathProbeRequest,
) -> Result<ValidatedOnboardingPaths, ServerFnError> {
    let home = server_home()?;
    md_web_services::domains::config_onboarding::validate_onboarding_paths(
        request,
        &onboarding_roots(&home),
        &home,
    )
    .map_err(|error| ServerFnError::new(format!("パスを登録できません: {error}")))
}

/// Loads real PostgreSQL configuration plus read-only host prerequisites.
#[get("/api/config/bootstrap")]
pub(crate) async fn config_bootstrap() -> Result<
    (
        PublicConfig,
        Vec<ToolStatus>,
        Vec<CapabilitySupport>,
        ReleaseRepository,
    ),
    ServerFnError,
> {
    #[cfg(feature = "server")]
    {
        let repository_adapter = repository().await?;
        let mut config =
            md_web_services::domains::config_onboarding::load_config(&repository_adapter)
                .await
                .map_err(|_| safe_error())?;
        if config.harness_home.is_none() {
            config.harness_home = Some(
                server_home()?
                    .canonicalize()
                    .map_err(|_| safe_error())?
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        overlay_provider_key_presence(&mut config).await?;
        Ok((
            config,
            md_web_services::domains::config_onboarding::probe_host_tools(None),
            md_web_services::domains::config_onboarding::web_capabilities(),
            release_repository()?,
        ))
    }
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

/// Returns the current browser-safe PostgreSQL snapshot.
#[get("/api/config")]
pub(crate) async fn config_get() -> Result<PublicConfig, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let repository_adapter = repository().await?;
        let mut config =
            md_web_services::domains::config_onboarding::load_config(&repository_adapter)
                .await
                .map_err(|_| safe_error())?;
        overlay_provider_key_presence(&mut config).await?;
        Ok(config)
    }
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

/// Stores a provider key server-side and returns only browser-safe presence metadata.
///
/// This adapter deliberately keeps plaintext out of browser DTOs and seals it through the
/// shared durable server-only secret provider.
#[post("/api/config/provider-key")]
pub(crate) async fn config_write_provider_key(
    request: ProviderKeyWrite,
) -> Result<SecretPresence, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let secret_id = SecretId::Provider(provider_secret_id(request.provider.as_str())?);
        let secret = WriteOnlySecret::new(String::from(request.key.expose_to_server()))
            .map_err(|_| safe_error())?;
        let provider = super::connections::hydrated_secret_provider().await?;
        let previous = provider.get_secret(&secret_id).map_err(|_| safe_error())?;
        provider
            .set_secret(secret_id.clone(), &secret)
            .map_err(|_| safe_error())?;
        if let Err(error) = super::connections::persist_connections_state().await {
            match previous {
                Some(previous) => {
                    let rollback = WriteOnlySecret::new(String::from(previous.expose_for_server()))
                        .map_err(|_| safe_error())?;
                    provider
                        .set_secret(secret_id, &rollback)
                        .map_err(|_| safe_error())?;
                }
                None => provider
                    .clear_secret(&secret_id)
                    .map_err(|_| safe_error())?,
            }
            return Err(error);
        }
        provider_key_presence().await
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

/// Applies one validated compare-and-swap configuration patch.
#[post("/api/config")]
pub(crate) async fn config_patch(patch: ConfigPatch) -> Result<PublicConfig, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let repository_adapter = repository().await?;
        md_web_services::domains::config_onboarding::patch_config(&repository_adapter, patch)
            .await
            .map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = patch;
        Err(safe_error())
    }
}

#[post("/api/config/agent-token-cap")]
pub(crate) async fn config_set_agent_token_cap(
    request: SetAgentTokenCapRequest,
) -> Result<ConfigRuntimeReceipt, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let repository_adapter = repository().await?;
        md_web_services::domains::config_onboarding::set_agent_token_cap(
            &repository_adapter,
            request,
        )
        .await
        .map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/config/change-home")]
pub(crate) async fn config_change_home(
    request: ChangeHomeRequest,
) -> Result<ConfigRuntimeReceipt, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let paths = validate_paths(&OnboardingPathProbeRequest {
            harness_home: request.harness_home,
            registered_repos: Vec::new(),
            workspace_cwd: None,
        })?;
        let repository_adapter = repository().await?;
        let receipt = md_web_services::domains::config_onboarding::change_home(
            &repository_adapter,
            ChangeHomeRequest {
                expected_revision: request.expected_revision,
                harness_home: paths.harness_home,
            },
        )
        .await
        .map_err(|_| safe_error())?;
        super::hive_reinitialize_harness_home().await;
        Ok(receipt)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

/// Persists the completed onboarding form as one CAS write.
#[post("/api/config/onboarding")]
pub(crate) async fn onboarding_finish(
    request: FinishOnboardingRequest,
) -> Result<FinishOnboardingResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let mut request = request;
        let paths = validate_paths(&OnboardingPathProbeRequest {
            harness_home: request.harness_home.clone(),
            registered_repos: request.registered_repos.clone(),
            workspace_cwd: Some(request.workspace_cwd.clone()),
        })?;
        request.harness_home = paths.harness_home;
        request.registered_repos = paths.registered_repos;
        request.workspace_cwd = paths
            .workspace_cwd
            .ok_or_else(|| ServerFnError::new("Gitワークスペースを確認できません"))?;
        let resolved_skills = super::memory::skills_local().await?;
        let repository_adapter = repository().await?;
        md_web_services::domains::config_onboarding::finish_onboarding(
            &repository_adapter,
            request,
            &resolved_skills,
        )
        .await
        .map_err(|_| safe_error())
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

/// Finalizes onboarding only after the runtime has observed all three team roles.
#[post("/api/config/onboarding/team-confirmed")]
pub(crate) async fn onboarding_confirm_team(
    request: ConfirmTeamInitializedRequest,
) -> Result<ConfirmTeamInitializedResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let repository = repository().await?;
        md_web_services::domains::config_onboarding::confirm_team_initialized(&repository, request)
            .await
            .map_err(|_| ServerFnError::new("初期チームの起動確認を保存できませんでした"))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

/// Canonicalizes an onboarding folder and Git workspaces without writing them.
#[post("/api/config/onboarding/paths")]
pub(crate) async fn onboarding_probe_paths(
    request: OnboardingPathProbeRequest,
) -> Result<ValidatedOnboardingPaths, ServerFnError> {
    #[cfg(feature = "server")]
    return validate_paths(&request);
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

/// Re-runs read-only PATH prerequisite probes on the server PC.
#[get("/api/config/prerequisites")]
pub(crate) async fn tools_status() -> Result<Vec<ToolStatus>, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(md_web_services::domains::config_onboarding::probe_host_tools(None));
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

/// Returns the initial update state without performing network I/O.
#[get("/api/config/update")]
pub(crate) async fn update_current() -> Result<UpdateStatus, ServerFnError> {
    Ok(UpdateStatus::Idle)
}

/// Reads the latest public release from the configured fork; it never installs anything.
#[post("/api/config/update/check")]
pub(crate) async fn update_check() -> Result<UpdateStatus, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let repository = release_repository()?;
        let client = md_web_services::domains::config_onboarding::GitHubReleaseClient::new()
            .map_err(|_| safe_error())?;
        md_web_services::domains::config_onboarding::check_for_update(
            &client,
            &repository,
            env!("CARGO_PKG_VERSION"),
        )
        .await
        .map_err(|_| ServerFnError::new("forkのリリース確認に失敗しました"))
    }
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

/// Reports explicit browser equivalents and Electron-only N/A capabilities.
#[get("/api/config/capabilities")]
pub(crate) async fn app_capabilities() -> Result<Vec<CapabilitySupport>, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(md_web_services::domains::config_onboarding::web_capabilities());
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

/// Returns the server build identity rather than an Electron process identity.
#[get("/api/config/app-info")]
pub(crate) async fn config_app_info() -> Result<AppInfo, ServerFnError> {
    #[cfg(feature = "server")]
    return Ok(md_web_services::domains::config_onboarding::app_info(
        env!("CARGO_PKG_VERSION"),
        "Dioxus Web版: PostgreSQL設定、fork更新確認、ブラウザーfloor",
    ));
    #[cfg(not(feature = "server"))]
    Err(safe_error())
}

/// Allocates an in-process floor namespace and returns a browser URL.
#[post("/api/config/floors")]
pub(crate) async fn floor_create(
    request: CreateFloorRequest,
) -> Result<CreateFloorResponse, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let _ = request;
        Err(ServerFnError::new(
            "floor別route/runtime namespaceが未接続のため利用できません",
        ))
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

/// Validates the observed PTY count, drains producers and closes PostgreSQL before shutdown.
#[post("/api/config/shutdown")]
pub(crate) async fn shutdown(request: ShutdownRequest) -> Result<ShutdownResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let running_terminals = super::pty::running_terminal_count()?;
        let mut decision = md_web_services::domains::config_onboarding::shutdown_decision(
            request,
            running_terminals,
        );
        if decision.accepted {
            decision.executed = super::shutdown_application(true).await.is_ok();
            decision.detail_ja = if decision.executed {
                String::from("すべてのproducerとPTYを停止し、PostgreSQL接続を閉じました。")
            } else {
                String::from("終了処理を完了できなかったため、サーバーを停止していません。")
            };
        }
        Ok(decision)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}

#[post("/api/config/reset")]
pub(crate) async fn reset_all(
    request: ResetNamespaceRequest,
) -> Result<ResetResult, ServerFnError> {
    #[cfg(feature = "server")]
    {
        let repository = repository().await?;
        md_web_services::domains::config_onboarding::validate_reset_request(&repository, &request)
            .map_err(|_| ServerFnError::new("namespaceの確認語句が一致しません"))?;
        let _reset_guard = super::begin_namespace_reset()
            .await
            .map_err(|_| ServerFnError::new("処理中のrequestを停止できませんでした"))?;
        let memory = super::memory::prepare_memory_namespace_reset().await?;
        if !memory.drained || memory.active_processes != 0 {
            let _ = super::memory::finish_memory_namespace_reset(false);
            return Err(ServerFnError::new("memory processを停止できませんでした"));
        }
        if let Err(error) = super::connections::prepare_connections_namespace_reset().await {
            let _ = super::memory::finish_memory_namespace_reset(false);
            return Err(error);
        }
        if super::pty::prepare_namespace_reset().await.is_err() {
            let _ = super::connections::reinitialize_connections_after_reset().await;
            let _ = super::memory::finish_memory_namespace_reset(false);
            return Err(ServerFnError::new("稼働中のPTYを停止できませんでした"));
        }
        let reset =
            md_web_services::domains::config_onboarding::reset_namespace(&repository, request)
                .await;
        let mut receipt = match reset {
            Ok(receipt) => receipt,
            Err(_) => {
                let _ = super::pty::finish_namespace_reset(false);
                let _ = super::connections::reinitialize_connections_after_reset().await;
                let _ = super::memory::finish_memory_namespace_reset(false);
                return Err(ServerFnError::new("namespaceを初期化できませんでした"));
            }
        };
        let local_reset = super::pty::finish_namespace_reset(true).is_ok()
            && super::office::reset_office_runtime_projection().is_ok()
            && super::memory::finish_memory_namespace_reset(true).is_ok();
        super::hive::hive_reinitialize_harness_home().await;
        let connections_reset = super::connections::reinitialize_connections_after_reset()
            .await
            .is_ok();
        if !(local_reset && connections_reset) {
            receipt.reset = false;
            receipt.detail_ja = String::from(
                "PostgreSQL namespaceは初期化しました。process状態を再読込できないためserverを再起動してください。",
            );
        }
        Ok(receipt)
    }
    #[cfg(not(feature = "server"))]
    {
        let _ = request;
        Err(safe_error())
    }
}
