use dioxus::prelude::*;
use md_web_contracts::{PersistenceCode, PersistenceStatus};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StatusPresentation {
    label: &'static str,
    detail: &'static str,
    tone: &'static str,
}

fn persistence_presentation(status: PersistenceStatus) -> StatusPresentation {
    match status {
        PersistenceStatus::Closed => StatusPresentation {
            label: "未接続",
            detail: "PostgreSQLへの接続は閉じています",
            tone: "idle",
        },
        PersistenceStatus::Ready { writes: true } => StatusPresentation {
            label: "利用可能",
            detail: "PostgreSQLへの読み書きが可能です",
            tone: "success",
        },
        PersistenceStatus::Ready { writes: false } => StatusPresentation {
            label: "読み取り専用",
            detail: "PostgreSQLへの書き込みは無効です",
            tone: "warning",
        },
        PersistenceStatus::Degraded { code } => degraded_presentation(code),
    }
}

fn degraded_presentation(code: PersistenceCode) -> StatusPresentation {
    match code {
        PersistenceCode::MissingConfig => StatusPresentation {
            label: "未設定",
            detail: "PostgreSQLの接続設定がありません",
            tone: "idle",
        },
        PersistenceCode::ConfigInvalid => StatusPresentation {
            label: "設定エラー",
            detail: "PostgreSQLの接続設定を確認してください",
            tone: "error",
        },
        PersistenceCode::Unreachable => StatusPresentation {
            label: "到達不能",
            detail: "PostgreSQLへ接続できません",
            tone: "error",
        },
        PersistenceCode::SchemaMismatch => StatusPresentation {
            label: "更新が必要",
            detail: "PostgreSQLのスキーマが一致しません",
            tone: "warning",
        },
        PersistenceCode::NamespaceLocked => StatusPresentation {
            label: "使用中",
            detail: "PostgreSQLの名前空間は別プロセスが使用中です",
            tone: "warning",
        },
        PersistenceCode::WriteFailed => StatusPresentation {
            label: "書込エラー",
            detail: "PostgreSQLへの書き込みに失敗しました",
            tone: "error",
        },
    }
}

#[component]
pub(crate) fn ServerStatusBadge(available: bool) -> Element {
    let (label, tone) = if available {
        ("サーバー稼働中", "success")
    } else {
        ("接続を確認中", "loading")
    };

    rsx! {
        span {
            class: "status-badge",
            "data-tone": tone,
            role: "status",
            span { class: "status-badge__dot", aria_hidden: "true" }
            span { {label} }
        }
    }
}

#[component]
pub(crate) fn PersistenceBadge(status: PersistenceStatus) -> Element {
    let presentation = persistence_presentation(status);

    rsx! {
        span {
            class: "status-badge",
            "data-tone": presentation.tone,
            title: presentation.detail,
            role: "status",
            span { class: "status-badge__dot", aria_hidden: "true" }
            span { {presentation.label} }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::persistence_presentation;
    use md_web_contracts::{PersistenceCode, PersistenceStatus};

    #[test]
    fn ready_writable_persistence_is_available() {
        let presentation = persistence_presentation(PersistenceStatus::Ready { writes: true });

        assert_eq!(presentation.label, "利用可能");
    }

    #[test]
    fn missing_configuration_is_distinct_from_unreachable() {
        let missing = persistence_presentation(PersistenceStatus::Degraded {
            code: PersistenceCode::MissingConfig,
        });
        let unreachable = persistence_presentation(PersistenceStatus::Degraded {
            code: PersistenceCode::Unreachable,
        });

        assert_ne!(missing.label, unreachable.label);
    }
}
