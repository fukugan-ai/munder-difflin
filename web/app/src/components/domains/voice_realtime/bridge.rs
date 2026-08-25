#![forbid(unsafe_code)]

use dioxus::prelude::*;
use md_web_contracts::domains::voice_realtime::{AudioDeviceKind, AudioDeviceView};

const VOICE_BRIDGE_JS: Asset = asset!("/src/components/domains/voice_realtime/realtime_bridge.js");

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BrowserVoiceCapabilities {
    pub secure_context: bool,
    pub devices: Vec<AudioDeviceView>,
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserRecording {
    pub target_agent_id: String,
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub filename: String,
}

#[component]
pub fn VoiceBridgeScript() -> Element {
    rsx! { document::Script { r#type: "module", src: VOICE_BRIDGE_JS } }
}

#[component]
pub fn VoiceBridgeEvents(on_event: EventHandler<serde_json::Value>) -> Element {
    use_effect(move || {
        spawn(async move {
            let mut evaluator = document::eval(
                r#"
                const send = (event) => dioxus.send(event.detail);
                globalThis.addEventListener("munder:voice-realtime-event", send);
                globalThis.addEventListener("munder:voice-freeflow-event", send);
                await new Promise(() => {});
                "#,
            );
            while let Ok(event) = evaluator.recv::<serde_json::Value>().await {
                on_event.call(event);
            }
        });
    });
    rsx! {}
}

pub async fn browser_voice_capabilities() -> Result<BrowserVoiceCapabilities, String> {
    let value = document::eval(
        r#"
        const bridge = globalThis.munderVoiceBridge;
        if (!bridge) return { secureContext: globalThis.isSecureContext === true, devices: [] };
        return {
          secureContext: bridge.isSecureContext(),
          devices: await bridge.enumerateDevices(),
          inputDeviceId: globalThis.localStorage?.getItem("munder.voice.inputDeviceId") || null,
          outputDeviceId: globalThis.localStorage?.getItem("munder.voice.outputDeviceId") || null,
        };
        "#,
    )
    .join::<serde_json::Value>()
    .await
    .map_err(|_| String::from("音声デバイスを確認できませんでした"))?;
    parse_capabilities(&value)
}

pub async fn browser_start_freeflow(
    target_agent_id: &str,
    input_device_id: Option<&str>,
) -> Result<(), String> {
    let target = json_literal(target_agent_id)?;
    let device = optional_json_literal(input_device_id)?;
    let script = format!(
        "await globalThis.munderVoiceBridge.startFreeflow({target}, {device}); return true;"
    );
    document::eval(&script)
        .join::<bool>()
        .await
        .map(|_| ())
        .map_err(|_| String::from("マイクを開始できませんでした"))
}

pub async fn browser_stop_freeflow() -> Result<BrowserRecording, String> {
    let value = document::eval(
        r#"
        const result = await globalThis.munderVoiceBridge.stopFreeflow();
        if (!result) return null;
        return {
          targetAgentId: result.targetAgentId,
          audioBytes: Array.from(new Uint8Array(result.audio)),
          mimeType: result.mimeType,
          filename: result.filename,
        };
        "#,
    )
    .join::<serde_json::Value>()
    .await
    .map_err(|_| String::from("録音を停止できませんでした"))?;
    parse_recording(&value)
}

pub async fn browser_connect_realtime(
    ephemeral_token: &str,
    input_device_id: Option<&str>,
    output_device_id: Option<&str>,
    idle_disconnect_ms: u64,
) -> Result<(), String> {
    let token = json_literal(ephemeral_token)?;
    let input = optional_json_literal(input_device_id)?;
    let output = optional_json_literal(output_device_id)?;
    let script = format!(
        "await globalThis.munderVoiceBridge.connectRealtime({{ ephemeralToken: {token}, inputDeviceId: {input}, outputDeviceId: {output}, idleMs: {idle_disconnect_ms} }}); return true;"
    );
    document::eval(&script)
        .join::<bool>()
        .await
        .map(|_| ())
        .map_err(|_| String::from("Realtime音声へ接続できませんでした"))
}

pub async fn browser_disconnect_realtime() -> Result<(), String> {
    document::eval("globalThis.munderVoiceBridge?.disconnectRealtime(); return true;")
        .join::<bool>()
        .await
        .map(|_| ())
        .map_err(|_| String::from("Realtime音声を終了できませんでした"))
}

pub async fn browser_set_output_device(device_id: Option<&str>) -> Result<(), String> {
    let device = optional_json_literal(device_id)?;
    let script =
        format!("await globalThis.munderVoiceBridge.setOutputDevice({device}); return true;");
    document::eval(&script)
        .join::<bool>()
        .await
        .map(|_| ())
        .map_err(|_| String::from("スピーカーを切り替えられませんでした"))
}

pub async fn browser_set_input_device(device_id: Option<&str>) -> Result<(), String> {
    let device = optional_json_literal(device_id)?;
    let script = format!("globalThis.munderVoiceBridge.setInputDevice({device}); return true;");
    document::eval(&script)
        .join::<bool>()
        .await
        .map(|_| ())
        .map_err(|_| String::from("マイク設定を保存できませんでした"))
}

pub async fn browser_configure_freeflow_shortcut(
    enabled: bool,
    target_agent_id: Option<&str>,
    input_device_id: Option<&str>,
) -> Result<(), String> {
    let target = optional_json_literal(target_agent_id)?;
    let input = optional_json_literal(input_device_id)?;
    let script = format!(
        "globalThis.munderVoiceBridge.configureFreeflowShortcut({{ enabled: {enabled}, targetAgentId: {target}, inputDeviceId: {input} }}); return true;"
    );
    document::eval(&script)
        .join::<bool>()
        .await
        .map(|_| ())
        .map_err(|_| String::from("Free Flowショートカットを設定できませんでした"))
}

pub async fn browser_send_realtime_tool_result(call_id: &str, output: &str) -> Result<(), String> {
    let call_id = json_literal(call_id)?;
    let output = json_literal(output)?;
    let script = format!(
        "return globalThis.munderVoiceBridge?.sendRealtimeToolResult({call_id}, {output}) === true;"
    );
    let sent = document::eval(&script)
        .join::<bool>()
        .await
        .map_err(|_| String::from("Realtime操作結果を返せませんでした"))?;
    if sent {
        Ok(())
    } else {
        Err(String::from("Realtime接続が終了しています"))
    }
}

pub async fn browser_send_realtime_notification(text: &str) -> Result<(), String> {
    let text = json_literal(text)?;
    let script = format!(
        r#"
        const bridge = globalThis.munderVoiceBridge;
        if (!bridge?.sendRealtimeEvent({{
          type: "conversation.item.create",
          item: {{ type: "message", role: "user", content: [{{ type: "input_text", text: {text} }}] }},
        }})) return false;
        return bridge.sendRealtimeEvent({{ type: "response.create" }});
        "#
    );
    let sent = document::eval(&script)
        .join::<bool>()
        .await
        .map_err(|_| String::from("Realtime通知を送れませんでした"))?;
    if sent {
        Ok(())
    } else {
        Err(String::from("Realtime接続が終了しています"))
    }
}

fn parse_capabilities(value: &serde_json::Value) -> Result<BrowserVoiceCapabilities, String> {
    let secure_context = value
        .get("secureContext")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let input_device_id = optional_string(value, "inputDeviceId");
    let output_device_id = optional_string(value, "outputDeviceId");
    let mut devices = Vec::new();
    for device in value
        .get("devices")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = device.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(label) = device.get("label").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let kind = match device.get("kind").and_then(serde_json::Value::as_str) {
            Some("input") => AudioDeviceKind::Input,
            Some("output") => AudioDeviceKind::Output,
            _ => continue,
        };
        devices.push(AudioDeviceView {
            id: String::from(id),
            label: String::from(label),
            kind,
        });
    }
    Ok(BrowserVoiceCapabilities {
        secure_context,
        devices,
        input_device_id,
        output_device_id,
    })
}

fn optional_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

fn parse_recording(value: &serde_json::Value) -> Result<BrowserRecording, String> {
    let target_agent_id = required_string(value, "targetAgentId")?;
    let mime_type = required_string(value, "mimeType")?;
    let filename = required_string(value, "filename")?;
    let values = value
        .get("audioBytes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| String::from("録音データがありません"))?;
    let mut bytes = Vec::with_capacity(values.len());
    for value in values {
        let byte = value
            .as_u64()
            .and_then(|byte| u8::try_from(byte).ok())
            .ok_or_else(|| String::from("録音データが不正です"))?;
        bytes.push(byte);
    }
    Ok(BrowserRecording {
        target_agent_id,
        bytes,
        mime_type,
        filename,
    })
}

fn required_string(value: &serde_json::Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .ok_or_else(|| String::from("音声ブリッジの応答が不正です"))
}

fn json_literal(value: &str) -> Result<String, String> {
    serde_json::to_string(value).map_err(|_| String::from("音声ブリッジ引数を作成できません"))
}

fn optional_json_literal(value: Option<&str>) -> Result<String, String> {
    serde_json::to_string(&value).map_err(|_| String::from("音声ブリッジ引数を作成できません"))
}

#[cfg(test)]
mod tests {
    use super::{json_literal, parse_capabilities, parse_recording};

    #[test]
    fn javascript_string_is_escaped() -> Result<(), String> {
        assert_eq!(json_literal("a\"b")?, r#""a\"b""#);
        Ok(())
    }

    #[test]
    fn capabilities_parse_devices() -> Result<(), String> {
        let value = serde_json::json!({
            "secureContext": true,
            "devices": [{ "id": "mic", "label": "Mic", "kind": "input" }]
        });
        let parsed = parse_capabilities(&value)?;

        assert!(parsed.secure_context);
        assert_eq!(parsed.devices.len(), 1);
        Ok(())
    }

    #[test]
    fn recording_rejects_out_of_range_byte() {
        let value = serde_json::json!({
            "targetAgentId": "agent",
            "audioBytes": [256],
            "mimeType": "audio/webm",
            "filename": "dictation.webm"
        });

        assert!(parse_recording(&value).is_err());
    }
}
