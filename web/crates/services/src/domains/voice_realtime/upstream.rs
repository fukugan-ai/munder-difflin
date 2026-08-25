#![forbid(unsafe_code)]

use std::time::Duration;

use md_web_contracts::domains::voice_realtime::{
    RealtimeMintResult, TranscriptionResult, VoiceErrorCode,
};
use reqwest::multipart::{Form, Part};

use super::ValidatedAudio;

const GROQ_TRANSCRIBE_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const OPENAI_CLIENT_SECRETS_URL: &str = "https://api.openai.com/v1/realtime/client_secrets";
const OPENAI_LEGACY_SESSIONS_URL: &str = "https://api.openai.com/v1/realtime/sessions";
const TRANSCRIBE_TIMEOUT: Duration = Duration::from_secs(60);
const MINT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VoiceUpstreamError {
    ClientBuild,
    InvalidMime,
}

#[derive(Clone, Debug)]
pub struct VoiceUpstreamClient {
    client: reqwest::Client,
}

impl VoiceUpstreamClient {
    pub fn new() -> Result<Self, VoiceUpstreamError> {
        let client = reqwest::Client::builder()
            .connect_timeout(MINT_TIMEOUT)
            .build()
            .map_err(|_| VoiceUpstreamError::ClientBuild)?;
        Ok(Self { client })
    }

    pub async fn transcribe(
        &self,
        api_key: &str,
        model: &str,
        audio: ValidatedAudio<'_>,
    ) -> TranscriptionResult {
        if api_key.trim().is_empty() {
            return transcription_error(VoiceErrorCode::NoKey, "Groq APIキーが未設定です");
        }
        let part = match Part::bytes(audio.bytes.to_vec())
            .file_name(String::from(audio.filename))
            .mime_str(audio.mime_type)
        {
            Ok(part) => part,
            Err(_) => {
                return transcription_error(
                    VoiceErrorCode::InvalidMime,
                    "音声形式を処理できません",
                );
            }
        };
        let mut form = Form::new()
            .text("model", String::from(model))
            .text("response_format", String::from("json"))
            .part("file", part);
        if let Some(language) = audio
            .language
            .filter(|language| !language.trim().is_empty())
        {
            form = form.text("language", String::from(language));
        }

        let response = self
            .client
            .post(GROQ_TRANSCRIBE_URL)
            .bearer_auth(api_key)
            .multipart(form)
            .timeout(TRANSCRIBE_TIMEOUT)
            .send()
            .await;
        let response = match response {
            Ok(response) => response,
            Err(error) if error.is_timeout() => {
                return transcription_error(
                    VoiceErrorCode::Timeout,
                    "文字起こしがタイムアウトしました",
                );
            }
            Err(_) => {
                return transcription_error(VoiceErrorCode::Upstream, "Groqへ接続できませんでした");
            }
        };
        let status = response.status();
        let body = match response.text().await {
            Ok(body) => body,
            Err(_) => {
                return transcription_error(
                    VoiceErrorCode::Upstream,
                    "Groqの応答を読み取れませんでした",
                );
            }
        };
        parse_transcription_response(status.as_u16(), &body)
    }

    pub async fn mint_realtime_token(&self, api_key: &str, model: &str) -> RealtimeMintResult {
        if api_key.trim().is_empty() {
            return mint_error(VoiceErrorCode::NoKey, "OpenAI APIキーが未設定です");
        }
        let first = self
            .post_mint(
                OPENAI_CLIENT_SECRETS_URL,
                api_key,
                serde_json::json!({ "session": { "type": "realtime", "model": model } }),
            )
            .await;
        let (status, body) = match first {
            Ok((404, _)) => match self
                .post_mint(
                    OPENAI_LEGACY_SESSIONS_URL,
                    api_key,
                    serde_json::json!({ "model": model }),
                )
                .await
            {
                Ok(response) => response,
                Err(result) => return result,
            },
            Ok(response) => response,
            Err(result) => return result,
        };
        parse_mint_response(status, &body, model)
    }

    async fn post_mint(
        &self,
        url: &str,
        api_key: &str,
        body: serde_json::Value,
    ) -> Result<(u16, String), RealtimeMintResult> {
        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .timeout(MINT_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    mint_error(
                        VoiceErrorCode::Timeout,
                        "トークン発行がタイムアウトしました",
                    )
                } else {
                    mint_error(VoiceErrorCode::Upstream, "OpenAIへ接続できませんでした")
                }
            })?;
        let status = response.status().as_u16();
        let body = response.text().await.map_err(|_| {
            mint_error(
                VoiceErrorCode::Upstream,
                "OpenAIの応答を読み取れませんでした",
            )
        })?;
        Ok((status, body))
    }
}

fn parse_transcription_response(status: u16, body: &str) -> TranscriptionResult {
    if !(200..300).contains(&status) {
        return transcription_error(
            VoiceErrorCode::Upstream,
            &format!("Groq文字起こしに失敗しました（HTTP {status}）"),
        );
    }
    let text = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("text")
                .and_then(|text| text.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| String::from(body))
        .trim()
        .to_owned();
    if text.is_empty() {
        transcription_error(VoiceErrorCode::NoAudio, "音声を検出できませんでした")
    } else {
        TranscriptionResult::Ok { text }
    }
}

fn parse_mint_response(status: u16, body: &str, model: &str) -> RealtimeMintResult {
    if !(200..300).contains(&status) {
        return mint_error(
            VoiceErrorCode::Upstream,
            &format!("Realtimeトークンを発行できませんでした（HTTP {status}）"),
        );
    }
    let value = match serde_json::from_str::<serde_json::Value>(body) {
        Ok(value) => value,
        Err(_) => return mint_error(VoiceErrorCode::Upstream, "OpenAIの応答形式が不正です"),
    };
    let client_secret = value.get("client_secret");
    let token = value
        .get("value")
        .and_then(|token| token.as_str())
        .or_else(|| {
            client_secret
                .and_then(|secret| secret.get("value"))
                .and_then(|token| token.as_str())
        });
    let Some(token) = token.filter(|token| !token.is_empty()) else {
        return mint_error(
            VoiceErrorCode::Upstream,
            "短期トークンが応答に含まれていません",
        );
    };
    let expires_at = value
        .get("expires_at")
        .and_then(|expires| expires.as_i64())
        .or_else(|| {
            client_secret
                .and_then(|secret| secret.get("expires_at"))
                .and_then(|expires| expires.as_i64())
        });
    RealtimeMintResult::Ok {
        ephemeral_token: String::from(token),
        expires_at,
        model: String::from(model),
    }
}

fn transcription_error(code: VoiceErrorCode, message: &str) -> TranscriptionResult {
    TranscriptionResult::Error {
        code,
        message: String::from(message),
    }
}

fn mint_error(code: VoiceErrorCode, message: &str) -> RealtimeMintResult {
    RealtimeMintResult::Error {
        code,
        message: String::from(message),
    }
}

#[cfg(test)]
mod tests {
    use md_web_contracts::domains::voice_realtime::{
        RealtimeMintResult, TranscriptionResult, VoiceErrorCode,
    };

    use super::{VoiceUpstreamClient, parse_mint_response, parse_transcription_response};

    #[test]
    fn client_builds_without_network() {
        assert!(VoiceUpstreamClient::new().is_ok());
    }

    #[test]
    fn transcription_parser_accepts_json() {
        let result = parse_transcription_response(200, r#"{"text":"hello"}"#);

        assert_eq!(
            result,
            TranscriptionResult::Ok {
                text: String::from("hello")
            }
        );
    }

    #[test]
    fn transcription_parser_redacts_upstream_body() {
        let result = parse_transcription_response(401, r#"{"error":"secret detail"}"#);

        assert!(
            matches!(result, TranscriptionResult::Error { code: VoiceErrorCode::Upstream, message } if !message.contains("secret"))
        );
    }

    #[test]
    fn mint_parser_accepts_ga_shape() {
        let result = parse_mint_response(200, r#"{"value":"ephemeral","expires_at":123}"#, "model");

        assert!(
            matches!(result, RealtimeMintResult::Ok { ephemeral_token, expires_at: Some(123), .. } if ephemeral_token == "ephemeral")
        );
    }

    #[test]
    fn mint_parser_requires_token() {
        let result = parse_mint_response(200, "{}", "model");

        assert!(matches!(
            result,
            RealtimeMintResult::Error {
                code: VoiceErrorCode::Upstream,
                ..
            }
        ));
    }
}
