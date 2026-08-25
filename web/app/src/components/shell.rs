use dioxus::prelude::*;

use dioxus_router::{Link, Outlet};

use crate::routes::{AppRoute, nav_items};

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
pub(crate) fn AppShell() -> Element {
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
                    for item in nav_items() {
                        if item.enabled {
                            Link {
                                class: "sidebar__item",
                                active_class: "is-active",
                                to: item.route,
                                span { aria_hidden: "true", {item.icon} }
                                span { {item.label} }
                            }
                        } else {
                            button {
                                class: "sidebar__item",
                                r#type: "button",
                                disabled: true,
                                "data-ui-state": "disabled",
                                title: "今後の移植で利用可能になります",
                                span { aria_hidden: "true", {item.icon} }
                                span { {item.label} }
                            }
                        }
                    }
                }

                main { id: "main-content", class: "route-main",
                    Outlet::<AppRoute> {}
                }
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
