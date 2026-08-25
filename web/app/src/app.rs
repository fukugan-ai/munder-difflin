use dioxus::prelude::*;

use dioxus_router::Router;

use crate::routes::AppRoute;

const APP_CSS: Asset = asset!("/assets/app.css");
const TOKENS_CSS: Asset = asset!("/assets/tokens.css");
const NOTO_SANS_JP_400: Asset = asset!("/assets/vendor/noto-sans-jp-japanese-400-normal.woff2");
const NOTO_SANS_JP_600: Asset = asset!("/assets/vendor/noto-sans-jp-japanese-600-normal.woff2");
const NOTO_SANS_JP_700: Asset = asset!("/assets/vendor/noto-sans-jp-japanese-700-normal.woff2");
const CONFIG_CSS: Asset = asset!("/assets/domains/config_onboarding.css");
const CONNECTIONS_CSS: Asset = asset!("/assets/domains/connections.css");
const FS_GIT_IDE_CSS: Asset = asset!("/assets/domains/fs_git_ide.css");
const HIVE_CSS: Asset = asset!("/assets/domains/hive_tasks.css");
const MEMORY_CSS: Asset = asset!("/assets/domains/memory_skills.css");
const OFFICE_CSS: Asset = asset!("/assets/domains/office_ui.css");
const PTY_CSS: Asset = asset!("/assets/domains/pty_agents.css");
const VOICE_CSS: Asset = asset!("/assets/domains/voice_realtime.css");

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SelectedAgentContext {
    pub(crate) agent_id: Option<String>,
    pub(crate) workspace_path: Option<String>,
}

#[component]
pub(crate) fn App() -> Element {
    let selected_agent = use_signal(SelectedAgentContext::default);
    use_context_provider(|| selected_agent);
    let noto_sans_jp = format!(
        r#"@font-face {{ font-family: "Noto Sans JP"; font-style: normal; font-display: swap; font-weight: 400; src: url("{NOTO_SANS_JP_400}") format("woff2"); }}
@font-face {{ font-family: "Noto Sans JP"; font-style: normal; font-display: swap; font-weight: 600; src: url("{NOTO_SANS_JP_600}") format("woff2"); }}
@font-face {{ font-family: "Noto Sans JP"; font-style: normal; font-display: swap; font-weight: 700; src: url("{NOTO_SANS_JP_700}") format("woff2"); }}"#
    );
    rsx! {
        document::Title { "Munder Difflin" }
        document::Style { {noto_sans_jp} }
        document::Link { rel: "stylesheet", href: TOKENS_CSS }
        document::Link { rel: "stylesheet", href: APP_CSS }
        document::Link { rel: "stylesheet", href: CONFIG_CSS }
        document::Link { rel: "stylesheet", href: CONNECTIONS_CSS }
        document::Link { rel: "stylesheet", href: FS_GIT_IDE_CSS }
        document::Link { rel: "stylesheet", href: HIVE_CSS }
        document::Link { rel: "stylesheet", href: MEMORY_CSS }
        document::Link { rel: "stylesheet", href: OFFICE_CSS }
        document::Link { rel: "stylesheet", href: PTY_CSS }
        document::Link { rel: "stylesheet", href: VOICE_CSS }
        Router::<AppRoute> {}
    }
}
