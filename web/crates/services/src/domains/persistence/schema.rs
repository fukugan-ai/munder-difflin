/// Schema version required by the Web parity persistence repository.
pub const EXPECTED_SCHEMA_VERSION: i32 = 2;

/// Forward-only migration text for an integration-owned migration runner.
pub const WEB_PARITY_MIGRATION_SQL: &str =
    include_str!("../../../../../../db/migrations/002_web_parity.sql");

#[cfg(test)]
mod tests {
    use super::{EXPECTED_SCHEMA_VERSION, WEB_PARITY_MIGRATION_SQL};

    #[test]
    fn expected_version_matches_migration_receipt() {
        assert!(WEB_PARITY_MIGRATION_SQL.contains(&format!(
            "schema_migrations(version) VALUES ({EXPECTED_SCHEMA_VERSION})"
        )));
    }

    #[test]
    fn migration_is_transaction_bounded() {
        assert!(
            WEB_PARITY_MIGRATION_SQL.starts_with("BEGIN;")
                && WEB_PARITY_MIGRATION_SQL.trim_end().ends_with("COMMIT;")
        );
    }

    #[test]
    fn migration_preserves_namespace_on_every_owned_table() {
        for table in [
            "web_app_config",
            "web_durable_records",
            "web_event_stream_heads",
            "web_event_replay",
            "web_trigger_history",
        ] {
            assert!(WEB_PARITY_MIGRATION_SQL.contains(table));
        }
    }
}
