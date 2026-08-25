use md_web_contracts::domains::config_onboarding::{ResetNamespaceRequest, ResetResult};

use crate::domains::persistence::PgPersistenceRepository;

use super::ConfigRepositoryError;
use super::config_repository::map_persistence_error;

/// Deletes only the confirmed application namespace in one PostgreSQL transaction.
pub async fn reset_namespace(
    repository: &PgPersistenceRepository,
    request: ResetNamespaceRequest,
) -> Result<ResetResult, ConfigRepositoryError> {
    validate_reset_request(repository, &request)?;
    let deleted_rows = repository
        .reset_bound_namespace()
        .await
        .map_err(map_persistence_error)?;
    Ok(ResetResult {
        reset: true,
        next_path: String::from("/onboarding"),
        detail_ja: String::from("確認したPostgreSQL namespaceをtransaction内で初期化しました。"),
        deleted_rows,
    })
}

/// Validates the typed confirmation against the repository-bound namespace
/// before any producer is stopped or transaction is opened.
pub fn validate_reset_request(
    repository: &PgPersistenceRepository,
    request: &ResetNamespaceRequest,
) -> Result<(), ConfigRepositoryError> {
    confirmation_matches(repository.namespace().as_str(), request)
        .then_some(())
        .ok_or(ConfigRepositoryError::InvalidRow)
}

fn confirmation_matches(namespace: &str, request: &ResetNamespaceRequest) -> bool {
    request.namespace == namespace && request.confirmation == format!("RESET {namespace}")
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::config_onboarding::ResetNamespaceRequest;
    use md_web_contracts::domains::persistence::Namespace;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

    use crate::domains::persistence::PgPersistenceRepository;

    use super::{confirmation_matches, validate_reset_request};

    #[test]
    fn reset_requires_exact_namespace_and_confirmation() {
        assert!(confirmation_matches(
            "local",
            &ResetNamespaceRequest {
                namespace: String::from("local"),
                confirmation: String::from("RESET local"),
            }
        ));
        assert!(!confirmation_matches(
            "local",
            &ResetNamespaceRequest {
                namespace: String::from("other"),
                confirmation: String::from("RESET local"),
            }
        ));
    }

    #[tokio::test]
    async fn reset_authority_comes_from_repository_namespace() -> Result<(), &'static str> {
        let pool = PgPoolOptions::new().connect_lazy_with(PgConnectOptions::new());
        let namespace =
            Namespace::parse(String::from("bound")).ok_or("test namespace should be valid")?;
        let repository = PgPersistenceRepository::new(pool, namespace);

        assert!(
            validate_reset_request(
                &repository,
                &ResetNamespaceRequest {
                    namespace: String::from("other"),
                    confirmation: String::from("RESET other"),
                }
            )
            .is_err()
        );
        assert!(
            validate_reset_request(
                &repository,
                &ResetNamespaceRequest {
                    namespace: String::from("bound"),
                    confirmation: String::from("RESET bound"),
                }
            )
            .is_ok()
        );
        Ok(())
    }
}
