//! Explicitly-started local connection transports.
//!
//! Nothing in this module starts at process boot. Callers must invoke `start`
//! from an operator action. The public tunnel is a child process so stop can
//! terminate both the listener and its tunnel without touching unrelated jobs.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md_web_contracts::domains::connections::{InboundKind, TriggerMode};
use serde_json::{Value, json};

const MAX_BODY_BYTES: usize = 1024 * 1024;
const REPLAY_WINDOW_SECONDS: u64 = 5 * 60;
const TUNNEL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct SlackRuntimeConfig {
    pub port: u16,
    pub signing_secret: String,
    pub channel_id: Option<String>,
}

#[derive(Clone)]
pub struct WebhookRuntimeEndpoint {
    pub id: String,
    pub name: String,
    pub secret: String,
    pub schema: String,
    pub mode: TriggerMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundWork {
    pub source_id: String,
    pub source_name: String,
    pub peer: String,
    pub title: Option<String>,
    pub body: String,
    pub kind: InboundKind,
    pub mode: TriggerMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlackInbound {
    pub channel: String,
    pub thread_ts: String,
    pub text: String,
}

pub type InboundDispatch = Arc<dyn Fn(InboundWork) -> Result<String, String> + Send + Sync>;
pub type SlackDispatch = Arc<dyn Fn(SlackInbound) -> Result<(), String> + Send + Sync>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStart {
    pub public_url: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuntimeKind {
    Slack,
    Webhooks,
    Broker,
}

struct ListenerHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    tunnel: Option<Child>,
}

impl ListenerHandle {
    fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(mut tunnel) = self.tunnel.take() {
            let _ = tunnel.kill();
            let _ = tunnel.wait();
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn registry() -> &'static Mutex<HashMap<RuntimeKind, ListenerHandle>> {
    static REGISTRY: OnceLock<Mutex<HashMap<RuntimeKind, ListenerHandle>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn start_slack(
    config: SlackRuntimeConfig,
    dispatch: SlackDispatch,
) -> Result<RuntimeStart, String> {
    let secret = config.signing_secret.clone();
    let channel_filter = config.channel_id.clone();
    start_listener(RuntimeKind::Slack, config.port, true, move |request| {
        if request.method != "POST" {
            return HttpResponse::empty(405);
        }
        if !verify_slack(&request, &secret) {
            return HttpResponse::empty(403);
        }
        let Ok(payload) = serde_json::from_slice::<Value>(&request.body) else {
            return HttpResponse::empty(400);
        };
        if payload.get("type").and_then(Value::as_str) == Some("url_verification") {
            return HttpResponse::json(
                200,
                json!({
                    "challenge": payload.get("challenge").and_then(Value::as_str).unwrap_or_default()
                }),
            );
        }
        let Some(event) = payload.get("event") else {
            return HttpResponse::empty(200);
        };
        let channel = event
            .get("channel")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if channel.is_empty()
            || channel_filter
                .as_deref()
                .is_some_and(|value| value != channel)
        {
            return HttpResponse::empty(200);
        }
        if event.get("bot_id").is_some() {
            return HttpResponse::empty(200);
        }
        let timestamp = event.get("ts").and_then(Value::as_str).unwrap_or_default();
        let thread_ts = event
            .get("thread_ts")
            .and_then(Value::as_str)
            .unwrap_or(timestamp);
        let text = strip_slack_mention(
            event
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        if !timestamp.is_empty() && !text.is_empty() {
            let _ = dispatch(SlackInbound {
                channel: channel.into(),
                thread_ts: thread_ts.into(),
                text,
            });
        }
        HttpResponse::empty(200)
    })
}

pub fn start_webhooks(
    port: u16,
    endpoints: Vec<WebhookRuntimeEndpoint>,
    dispatch: InboundDispatch,
) -> Result<RuntimeStart, String> {
    start_listener(RuntimeKind::Webhooks, port, true, move |request| {
        let id = request
            .path
            .split('?')
            .next()
            .unwrap_or_default()
            .trim_matches('/');
        let endpoint = endpoints.iter().find(|item| item.id == id);
        if request.method == "GET" {
            return HttpResponse::json(200, json!({ "ok": true, "endpoint": id }));
        }
        if request.method != "POST" {
            return HttpResponse::empty(405);
        }
        let provided = request.header("x-md-webhook-secret").unwrap_or_default();
        let Some(endpoint) =
            endpoint.filter(|item| constant_time_eq(provided.as_bytes(), item.secret.as_bytes()))
        else {
            return HttpResponse::json(401, json!({ "ok": false, "error": "unauthorized" }));
        };
        let Ok(payload) = serde_json::from_slice::<Value>(&request.body) else {
            return HttpResponse::json(400, json!({ "ok": false, "error": "bad json" }));
        };
        let Some(message) = payload
            .get("message")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return HttpResponse::json(400, json!({ "ok": false, "error": "message required" }));
        };
        if endpoint.schema.contains("\"required\"") && !endpoint.schema.contains("\"message\"") {
            return HttpResponse::json(
                500,
                json!({ "ok": false, "error": "invalid endpoint schema" }),
            );
        }
        let kind = match payload.get("kind").and_then(Value::as_str) {
            Some("communication") => InboundKind::Communication,
            _ => InboundKind::Directive,
        };
        let work = InboundWork {
            source_id: endpoint.id.clone(),
            source_name: endpoint.name.clone(),
            peer: payload
                .get("from")
                .and_then(Value::as_str)
                .unwrap_or(&endpoint.name)
                .into(),
            title: payload
                .get("title")
                .and_then(Value::as_str)
                .map(String::from),
            body: message.into(),
            kind,
            mode: endpoint.mode,
        };
        match dispatch(work) {
            Ok(task_id) if task_id.is_empty() => {
                HttpResponse::json(202, json!({ "ok": true, "pending": true }))
            }
            Ok(task_id) => HttpResponse::json(200, json!({ "ok": true, "taskId": task_id })),
            Err(_) => HttpResponse::json(500, json!({ "ok": false, "error": "dispatch failed" })),
        }
    })
}

pub fn start_broker(
    port: u16,
    handler: Arc<dyn Fn(HttpRequest) -> HttpResponse + Send + Sync>,
) -> Result<RuntimeStart, String> {
    start_listener(RuntimeKind::Broker, port, false, move |request| {
        handler(request)
    })
}

pub fn stop(kind: RuntimeKind) -> Result<(), String> {
    let handle = registry()
        .lock()
        .map_err(|_| String::from("runtime registry unavailable"))?
        .remove(&kind);
    if let Some(handle) = handle {
        handle.stop();
    }
    Ok(())
}

fn start_listener<F>(
    kind: RuntimeKind,
    port: u16,
    tunnel: bool,
    handler: F,
) -> Result<RuntimeStart, String>
where
    F: Fn(HttpRequest) -> HttpResponse + Send + Sync + 'static,
{
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let mut guard = registry()
        .lock()
        .map_err(|_| String::from("runtime registry unavailable"))?;
    if guard.contains_key(&kind) {
        return Err(String::from("listener is already running"));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let handler = Arc::new(handler);
    let thread = thread::Builder::new()
        .name(format!("connections-{kind:?}"))
        .spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let response = read_request(&mut stream)
                            .map_or_else(|_| HttpResponse::empty(400), |request| handler(request));
                        let _ = response.write_to(&mut stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        })
        .map_err(|error| error.to_string())?;

    let (tunnel_child, public_url, detail) = if tunnel {
        match open_tunnel(port) {
            Ok((child, url)) => (Some(child), Some(url), None),
            Err(error) => (
                None,
                Some(format!("http://127.0.0.1:{port}")),
                Some(format!(
                    "local listener is running; tunnel unavailable: {error}"
                )),
            ),
        }
    } else {
        (None, Some(format!("http://127.0.0.1:{port}")), None)
    };
    guard.insert(
        kind,
        ListenerHandle {
            stop,
            thread: Some(thread),
            tunnel: tunnel_child,
        },
    );
    Ok(RuntimeStart { public_url, detail })
}

fn open_tunnel(port: u16) -> Result<(Child, String), String> {
    let executable = std::env::var_os("MD_TUNNELMOLE_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            let local = std::path::PathBuf::from("node_modules/.bin/tunnelmole");
            local.is_file().then_some(local)
        })
        .unwrap_or_else(|| std::path::PathBuf::from("tmole"));
    let mut child = Command::new(executable)
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| String::from("tunnel stdout unavailable"))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(url) = line
                .split_whitespace()
                .find(|word| word.starts_with("https://"))
            {
                let _ = sender.send(url.trim_end_matches(['.', ',']).to_owned());
                break;
            }
        }
    });
    match receiver.recv_timeout(TUNNEL_TIMEOUT) {
        Ok(url) => Ok((child, url)),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(String::from("timed out waiting for tunnel URL"))
        }
    }
}

#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
}

impl HttpResponse {
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: "text/plain",
            body: Vec::new(),
        }
    }

    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(&value).unwrap_or_default(),
        }
    }

    pub fn raw(status: u16, content_type: &'static str, body: Vec<u8>) -> Self {
        Self {
            status,
            content_type,
            body,
        }
    }

    fn write_to(&self, stream: &mut TcpStream) -> std::io::Result<()> {
        let reason = match self.status {
            200 => "OK",
            202 => "Accepted",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            413 => "Payload Too Large",
            _ => "Internal Server Error",
        };
        write!(
            stream,
            "HTTP/1.1 {} {}\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            self.status,
            reason,
            self.content_type,
            self.body.len()
        )?;
        stream.write_all(&self.body)
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err(String::from("incomplete request"));
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > MAX_BODY_BYTES + 16 * 1024 {
            return Err(String::from("request too large"));
        }
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let header_text =
        std::str::from_utf8(&bytes[..header_end]).map_err(|_| String::from("invalid headers"))?;
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    if method.is_empty() || path.is_empty() {
        return Err(String::from("invalid request line"));
    }
    let mut headers = HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(String::from("body too large"));
    }
    while bytes.len().saturating_sub(header_end) < content_length {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Err(String::from("incomplete body"));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn verify_slack(request: &HttpRequest, secret: &str) -> bool {
    let Some(timestamp) = request.header("x-slack-request-timestamp") else {
        return false;
    };
    let Some(signature) = request.header("x-slack-signature") else {
        return false;
    };
    let Ok(timestamp_number) = timestamp.parse::<u64>() else {
        return false;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    if now.abs_diff(timestamp_number) > REPLAY_WINDOW_SECONDS {
        return false;
    }
    let mut message = format!("v0:{timestamp}:").into_bytes();
    message.extend_from_slice(&request.body);
    let expected = format!("v0={}", hex(&hmac_sha256(secret.as_bytes(), &message)));
    constant_time_eq(expected.as_bytes(), signature.as_bytes())
}

fn strip_slack_mention(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("<@")
        && let Some((_, tail)) = rest.split_once('>')
    {
        return tail.trim().to_owned();
    }
    trimmed.to_owned()
}

pub(super) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

pub(super) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut normalized = [0_u8; 64];
    if key.len() > 64 {
        normalized[..32].copy_from_slice(&sha256(key));
    } else {
        normalized[..key.len()].copy_from_slice(key);
    }
    let mut inner = [0x36_u8; 64];
    let mut outer = [0x5c_u8; 64];
    for index in 0..64 {
        inner[index] ^= normalized[index];
        outer[index] ^= normalized[index];
    }
    let mut inner_input = inner.to_vec();
    inner_input.extend_from_slice(message);
    let inner_hash = sha256(&inner_input);
    let mut outer_input = outer.to_vec();
    outer_input.extend_from_slice(&inner_hash);
    sha256(&outer_input)
}

pub(super) fn sha256(input: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    let mut state = INITIAL;
    let (chunks, remainder) = padded.as_chunks::<64>();
    debug_assert!(remainder.is_empty());
    for chunk in chunks {
        let mut words = [0_u32; 64];
        for (index, word) in words.iter_mut().take(16).enumerate() {
            *word =
                u32::from_be_bytes(chunk[index * 4..index * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let mut work = state;
        for index in 0..64 {
            let s1 = work[4].rotate_right(6) ^ work[4].rotate_right(11) ^ work[4].rotate_right(25);
            let choice = (work[4] & work[5]) ^ (!work[4] & work[6]);
            let temp1 = work[7]
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let s0 = work[0].rotate_right(2) ^ work[0].rotate_right(13) ^ work[0].rotate_right(22);
            let majority = (work[0] & work[1]) ^ (work[0] & work[2]) ^ (work[1] & work[2]);
            let temp2 = s0.wrapping_add(majority);
            work = [
                temp1.wrapping_add(temp2),
                work[0],
                work[1],
                work[2],
                work[3].wrapping_add(temp1),
                work[4],
                work[5],
                work[6],
            ];
        }
        for index in 0..8 {
            state[index] = state[index].wrapping_add(work[index]);
        }
    }
    let mut output = [0_u8; 32];
    for (index, word) in state.into_iter().enumerate() {
        output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, hex, hmac_sha256, sha256, strip_slack_mention};

    #[test]
    fn sha256_matches_known_vector() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_matches_known_vector() {
        assert_eq!(
            hex(&hmac_sha256(
                b"key",
                b"The quick brown fox jumps over the lazy dog"
            )),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn comparisons_and_mentions_are_bounded() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"short", b"longer"));
        assert_eq!(strip_slack_mention(" <@BOT123>  do work "), "do work");
    }
}
