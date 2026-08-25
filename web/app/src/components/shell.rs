use dioxus::prelude::*;

use super::dashboard::{Dashboard, HealthViewState};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ThemePreference {
    System,
    Dark,
    Light,
}

impl ThemePreference {
    fn next(self) -> Self {
        match self {
            Self::System => Self::Dark,
            Self::Dark => Self::Light,
            Self::Light => Self::System,
        }
    }

    fn attribute(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::System => "システム設定",
            Self::Dark => "ダーク",
            Self::Light => "ライト",
        }
    }

    fn glyph(self) -> &'static str {
        match self {
            Self::System => "◐",
            Self::Dark => "☾",
            Self::Light => "☀",
        }
    }
}

#[component]
pub(crate) fn AppShell(health: HealthViewState, on_refresh: EventHandler<()>) -> Element {
    let mut theme = use_signal(|| ThemePreference::System);
    let current_theme = *theme.read();

    rsx! {
        div {
            class: "app-shell",
            "data-theme": current_theme.attribute(),
            a { class: "skip-link", href: "#main-content", "本文へ移動" }

            header { class: "app-header",
                div { class: "brand",
                    span { class: "brand__mark", aria_hidden: "true", "MD" }
                    span { class: "brand__name", "MUNDER DIFFLIN" }
                    span { class: "brand__edition", "ローカルWeb版" }
                }
                button {
                    class: "ui-button ui-button--theme",
                    r#type: "button",
                    "data-ui-state": "default",
                    aria_label: "表示テーマを切り替える",
                    aria_pressed: (current_theme != ThemePreference::System).to_string(),
                    title: format!("表示テーマ：{}", current_theme.label()),
                    onclick: move |_| {
                        let next = theme.read().next();
                        theme.set(next);
                    },
                    span { aria_hidden: "true", {current_theme.glyph()} }
                    span { class: "theme-label", {current_theme.label()} }
                }
            }

            div { class: "app-layout",
                nav { class: "sidebar", aria_label: "主要メニュー",
                    p { class: "sidebar__label", "ワークスペース" }
                    a {
                        class: "sidebar__item is-active",
                        href: "#office-title",
                        aria_current: "page",
                        span { aria_hidden: "true", "▦" }
                        span { "オフィス" }
                    }
                    for (label, icon) in [
                        ("ターミナル", ">_"),
                        ("タスク", "✓"),
                        ("確認事項", "!"),
                        ("トリガー", "◷"),
                        ("履歴", "≡"),
                        ("記憶", "✦"),
                        ("ワーカー", "⚙"),
                    ] {
                        button {
                            class: "sidebar__item",
                            r#type: "button",
                            disabled: true,
                            "data-ui-state": "disabled",
                            title: "今後の移植で利用可能になります",
                            span { aria_hidden: "true", {icon} }
                            span { {label} }
                        }
                    }
                }

                Dashboard { health, on_refresh }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ThemePreference;

    #[test]
    fn theme_cycle_returns_to_system_preference() {
        let cycled = ThemePreference::System.next().next().next();

        assert_eq!(cycled, ThemePreference::System);
    }
}
