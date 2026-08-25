//! Durable, browser-safe metadata and authenticated encrypted secret envelopes.

use std::io::Read;

use md_web_contracts::domains::connections::{ConnectionsSnapshot, RuntimeStatus};
use serde::{Deserialize, Serialize};

use super::runtime::{constant_time_eq, hex, hmac_sha256, sha256};
use super::{
    ConnectionsServiceError, DomainState, ProviderSecretId, SecretId, VoiceDurableSettings,
    stopped_listener,
};

const DOCUMENT_VERSION: u8 = 1;
const SECRET_ENVELOPE_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HydrationPlan {
    pub restart_slack: bool,
    pub restart_webhooks: bool,
    pub restart_broker: bool,
}

#[derive(Deserialize, Serialize)]
struct DurableDocument {
    version: u8,
    snapshot: ConnectionsSnapshot,
    desired: DesiredRuntimes,
    context_last_fired: [Option<u64>; 2],
    #[serde(default)]
    voice_settings: VoiceDurableSettings,
}

#[derive(Deserialize, Serialize)]
struct DesiredRuntimes {
    slack: bool,
    webhooks: bool,
    broker: bool,
}

#[derive(Deserialize, Serialize)]
struct SecretRecord {
    id: String,
    value: String,
}

#[derive(Deserialize, Serialize)]
struct SecretEnvelope {
    version: u8,
    nonce: String,
    ciphertext: String,
    tag: String,
}

pub(super) fn encode_metadata(state: &DomainState) -> Result<String, ConnectionsServiceError> {
    let snapshot = state.snapshot();
    serde_json::to_string(&DurableDocument {
        version: DOCUMENT_VERSION,
        desired: DesiredRuntimes {
            slack: matches!(snapshot.slack.listener.state, RuntimeStatus::Running),
            webhooks: matches!(snapshot.webhook_listener.state, RuntimeStatus::Running),
            broker: matches!(snapshot.broker.state, RuntimeStatus::Running),
        },
        context_last_fired: state.context_last_fired,
        voice_settings: state.voice_settings.clone(),
        snapshot,
    })
    .map_err(|_| ConnectionsServiceError::InvalidData("connections metadata"))
}

pub(super) fn hydrate_metadata(
    state: &mut DomainState,
    encoded: &str,
) -> Result<HydrationPlan, ConnectionsServiceError> {
    let mut document: DurableDocument = serde_json::from_str(encoded)
        .map_err(|_| ConnectionsServiceError::InvalidData("connections metadata"))?;
    if document.version != DOCUMENT_VERSION {
        return Err(ConnectionsServiceError::InvalidData(
            "connections metadata version",
        ));
    }
    let plan = HydrationPlan {
        restart_slack: document.desired.slack,
        restart_webhooks: document.desired.webhooks,
        restart_broker: document.desired.broker,
    };
    document.snapshot.slack.listener = stopped_listener();
    document.snapshot.webhook_listener = stopped_listener();
    document.snapshot.broker = stopped_listener();
    state.apply_snapshot(document.snapshot);
    state.context_last_fired = document.context_last_fired;
    state.voice_settings = document.voice_settings;
    state.sync_secret_flags();
    Ok(plan)
}

pub(super) fn seal_secrets(
    state: &DomainState,
    master_key: &[u8],
) -> Result<String, ConnectionsServiceError> {
    validate_master_key(master_key)?;
    let records: Vec<SecretRecord> = state
        .secrets
        .values
        .iter()
        .map(|(id, value)| SecretRecord {
            id: encode_secret_id(id),
            value: value.clone(),
        })
        .collect();
    let plaintext = serde_json::to_vec(&records)
        .map_err(|_| ConnectionsServiceError::InvalidData("secret bundle"))?;
    let key = sha256(master_key);
    let nonce = random_nonce()?;
    let ciphertext = chacha20_xor(&key, &nonce, &plaintext);
    let mut authenticated = vec![SECRET_ENVELOPE_VERSION];
    authenticated.extend_from_slice(&nonce);
    authenticated.extend_from_slice(&ciphertext);
    let tag = hmac_sha256(&key, &authenticated);
    serde_json::to_string(&SecretEnvelope {
        version: SECRET_ENVELOPE_VERSION,
        nonce: hex(&nonce),
        ciphertext: hex(&ciphertext),
        tag: hex(&tag),
    })
    .map_err(|_| ConnectionsServiceError::InvalidData("secret envelope"))
}

pub(super) fn open_secrets(
    state: &mut DomainState,
    master_key: &[u8],
    encoded: &str,
) -> Result<(), ConnectionsServiceError> {
    validate_master_key(master_key)?;
    let envelope: SecretEnvelope = serde_json::from_str(encoded)
        .map_err(|_| ConnectionsServiceError::InvalidData("secret envelope"))?;
    if envelope.version != SECRET_ENVELOPE_VERSION {
        return Err(ConnectionsServiceError::InvalidData(
            "secret envelope version",
        ));
    }
    let nonce = decode_hex(&envelope.nonce)?;
    let ciphertext = decode_hex(&envelope.ciphertext)?;
    let provided_tag = decode_hex(&envelope.tag)?;
    if nonce.len() != 12 || provided_tag.len() != 32 {
        return Err(ConnectionsServiceError::InvalidData("secret envelope"));
    }
    let key = sha256(master_key);
    let mut authenticated = vec![envelope.version];
    authenticated.extend_from_slice(&nonce);
    authenticated.extend_from_slice(&ciphertext);
    let expected_tag = hmac_sha256(&key, &authenticated);
    if !constant_time_eq(&provided_tag, &expected_tag) {
        return Err(ConnectionsServiceError::InvalidData("secret envelope tag"));
    }
    let nonce: [u8; 12] = nonce
        .try_into()
        .map_err(|_| ConnectionsServiceError::InvalidData("secret nonce"))?;
    let plaintext = chacha20_xor(&key, &nonce, &ciphertext);
    let records: Vec<SecretRecord> = serde_json::from_slice(&plaintext)
        .map_err(|_| ConnectionsServiceError::InvalidData("secret bundle"))?;
    state.secrets.values.clear();
    for record in records {
        let id = decode_secret_id(&record.id)?;
        if record.value.trim().is_empty() {
            return Err(ConnectionsServiceError::InvalidData("blank secret"));
        }
        state.secrets.values.insert(id, record.value);
    }
    state.sync_secret_flags();
    Ok(())
}

fn validate_master_key(master_key: &[u8]) -> Result<(), ConnectionsServiceError> {
    if master_key.len() < 32 {
        return Err(ConnectionsServiceError::MissingSecret(
            "connections master key",
        ));
    }
    Ok(())
}

fn encode_secret_id(id: &SecretId) -> String {
    match id {
        SecretId::SlackSigning => String::from("slack:signing"),
        SecretId::SlackBot => String::from("slack:bot"),
        SecretId::Webhook(id) => format!("webhook:{id}"),
        SecretId::Integration(id) => format!("integration:{id}"),
        SecretId::Organisation => String::from("organisation:key"),
        SecretId::Provider(ProviderSecretId::OpenAi) => String::from("provider:openai"),
        SecretId::Provider(ProviderSecretId::Groq) => String::from("provider:groq"),
    }
}

fn decode_secret_id(encoded: &str) -> Result<SecretId, ConnectionsServiceError> {
    match encoded {
        "slack:signing" => Ok(SecretId::SlackSigning),
        "slack:bot" => Ok(SecretId::SlackBot),
        "organisation:key" => Ok(SecretId::Organisation),
        "provider:openai" => Ok(SecretId::Provider(ProviderSecretId::OpenAi)),
        "provider:groq" => Ok(SecretId::Provider(ProviderSecretId::Groq)),
        _ if encoded.starts_with("webhook:") => Ok(SecretId::Webhook(
            encoded.trim_start_matches("webhook:").to_owned(),
        )),
        _ if encoded.starts_with("integration:") => Ok(SecretId::Integration(
            encoded.trim_start_matches("integration:").to_owned(),
        )),
        _ => Err(ConnectionsServiceError::InvalidData("secret id")),
    }
}

fn random_nonce() -> Result<[u8; 12], ConnectionsServiceError> {
    let mut nonce = [0_u8; 12];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut random| random.read_exact(&mut nonce))
        .map_err(|_| ConnectionsServiceError::Runtime(String::from("secure random unavailable")))?;
    Ok(nonce)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, ConnectionsServiceError> {
    if !value.len().is_multiple_of(2) {
        return Err(ConnectionsServiceError::InvalidData("hex envelope"));
    }
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ConnectionsServiceError::InvalidData("hex envelope"));
    }
    pairs
        .iter()
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(value: u8) -> Result<u8, ConnectionsServiceError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(ConnectionsServiceError::InvalidData("hex envelope")),
    }
}

fn chacha20_xor(key: &[u8; 32], nonce: &[u8; 12], input: &[u8]) -> Vec<u8> {
    let constants = *b"expand 32-byte k";
    let mut output = Vec::with_capacity(input.len());
    for (block_index, chunk) in input.chunks(64).enumerate() {
        let counter = u32::try_from(block_index + 1).unwrap_or(u32::MAX);
        let block = chacha20_block(&constants, key, counter, nonce);
        output.extend(chunk.iter().zip(block).map(|(byte, mask)| byte ^ mask));
    }
    output
}

fn chacha20_block(
    constants: &[u8; 16],
    key: &[u8; 32],
    counter: u32,
    nonce: &[u8; 12],
) -> [u8; 64] {
    let mut state = [0_u32; 16];
    for (index, word) in constants.as_chunks::<4>().0.iter().enumerate() {
        state[index] = u32::from_le_bytes(*word);
    }
    for (index, word) in key.as_chunks::<4>().0.iter().enumerate() {
        state[index + 4] = u32::from_le_bytes(*word);
    }
    state[12] = counter;
    for (index, word) in nonce.as_chunks::<4>().0.iter().enumerate() {
        state[index + 13] = u32::from_le_bytes(*word);
    }
    let mut working = state;
    for _ in 0..10 {
        quarter_round(&mut working, 0, 4, 8, 12);
        quarter_round(&mut working, 1, 5, 9, 13);
        quarter_round(&mut working, 2, 6, 10, 14);
        quarter_round(&mut working, 3, 7, 11, 15);
        quarter_round(&mut working, 0, 5, 10, 15);
        quarter_round(&mut working, 1, 6, 11, 12);
        quarter_round(&mut working, 2, 7, 8, 13);
        quarter_round(&mut working, 3, 4, 9, 14);
    }
    let mut output = [0_u8; 64];
    for (index, value) in working.into_iter().enumerate() {
        output[index * 4..index * 4 + 4]
            .copy_from_slice(&value.wrapping_add(state[index]).to_le_bytes());
    }
    output
}

fn quarter_round(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(12);
    state[a] = state[a].wrapping_add(state[b]);
    state[d] = (state[d] ^ state[a]).rotate_left(8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = (state[b] ^ state[c]).rotate_left(7);
}
