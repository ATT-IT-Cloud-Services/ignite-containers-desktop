//! Background API transport for the desktop client.
//!
//! The UI thread never performs network IO. Every request is handed to a worker
//! thread that owns a small Tokio runtime; results are returned over an
//! `std::sync::mpsc` channel and the egui context is repainted. The console
//! poller is a second, independent thread so a slow log request can never block
//! a power action (or the UI).

use egui::Context;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

/// Connection settings. The API key is never printed by the `Debug` impl.
#[derive(Clone)]
pub struct Connection {
    pub base_url: String,
    pub api_key: String,
}

impl Connection {
    #[must_use]
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    #[must_use]
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }
}

impl fmt::Debug for Connection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Connection")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

/// Identifies what a response is for. Mutating tags remember enough context to
/// refresh the right part of the UI without re-parsing the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tag {
    Connect,
    Me,
    Servers,
    ServerDetail(String),
    ServerStats(String),
    Nodes,
    Allocations(String),
    Templates,
    TemplateDetail(String),
    FilesList {
        server: String,
        path: String,
    },
    FileRead {
        server: String,
        path: String,
    },
    DbHosts,
    ServerDbs(String),
    Backups(String),
    Audit,
    Proxy,
    Raw,
    ServerCreate,
    ServerUpdate {
        server: String,
    },
    ServerPower {
        server: String,
        action: &'static str,
    },
    ConsoleCommand {
        server: String,
    },
    ServerReinstall {
        server: String,
    },
    ServerSuspend {
        server: String,
        resume: bool,
    },
    ServerDelete {
        server: String,
    },
    NodeCreate,
    AllocationCreate {
        node: String,
    },
    AllocationDelete {
        node: String,
        id: String,
    },
    TemplateImport,
    TemplateDelete {
        id: String,
    },
    FileSave {
        server: String,
    },
    FileMkdir {
        server: String,
    },
    FileRename {
        server: String,
    },
    FileChmod {
        server: String,
    },
    FileDelete {
        server: String,
    },
    FileCompress {
        server: String,
    },
    FileDecompress {
        server: String,
    },
    DbHostCreate,
    DbHostDelete {
        id: String,
    },
    DbHostTest {
        id: String,
    },
    DbCreate {
        server: String,
    },
    DbDelete {
        server: String,
        id: String,
    },
    BackupCreate {
        server: String,
    },
    BackupDelete {
        server: String,
        id: String,
    },
    BackupRestore {
        server: String,
        id: String,
    },
    BackupDownload {
        server: String,
        id: String,
    },
    ProxyCreate,
    ProxyDelete {
        id: String,
    },
    /// Read-only arbitrary API response, stored under a UI feature key.
    Feature(String),
    /// Mutating arbitrary API response; refreshes a feature key after success.
    FeatureMutation {
        key: String,
        refresh: String,
        refresh_path: Option<String>,
        success: String,
        result_key: Option<String>,
    },
    NodeUpdate {
        id: String,
    },
    NodeRotate {
        id: String,
    },
    NodeDelete {
        id: String,
    },
    NodeUpdatesCheck {
        id: String,
    },
    NodeUpdatesApply {
        id: String,
        reboot: bool,
    },
    NodeReboot {
        id: String,
    },
    TemplatePublish {
        id: String,
    },
    TemplateFeed,
    TemplateAutoUpdate {
        id: String,
    },
    ProxyUpdate {
        id: String,
    },
    ProxyReconcile,
    BulkPower,
    BulkCommand,
    AllocationAttach {
        server: String,
        allocation: String,
    },
    AllocationDetach {
        server: String,
        allocation: String,
    },
}

pub struct ApiRequest {
    pub tag: Tag,
    pub conn: Connection,
    pub method: String,
    pub path: String,
    pub body: Option<Value>,
    pub download: Option<PathBuf>,
}

pub struct ApiResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: String,
}

impl ApiResponse {
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn json<T: DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_str(&self.body)
            .map_err(|error| format!("could not decode response: {error}"))
    }

    #[must_use]
    pub fn error_message(&self) -> String {
        api_error_message(self.status, &self.body)
    }
}

pub struct ApiResult {
    pub tag: Tag,
    pub outcome: Result<ApiResponse, String>,
}

/// Turn the control plane error envelope into a short, user-facing message.
#[must_use]
pub fn api_error_message(status: u16, body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            let code = value
                .pointer("/error/code")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return if code.is_empty() {
                format!("HTTP {status}: {message}")
            } else {
                format!("HTTP {status} ({code}): {message}")
            };
        }
    }
    let snippet: String = body.chars().take(300).collect();
    let snippet = snippet.trim();
    if snippet.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {snippet}")
    }
}

pub struct ApiWorker {
    tx: tokio::sync::mpsc::UnboundedSender<ApiRequest>,
    rx: mpsc::Receiver<ApiResult>,
}

impl ApiWorker {
    pub fn new(ctx: Context) -> Self {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ApiRequest>();
        let (out_tx, out_rx) = mpsc::channel::<ApiResult>();

        std::thread::Builder::new()
            .name("ignite-api".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .thread_name("ignite-api-io")
                    .build()
                    .expect("failed to start API worker runtime");

                runtime.block_on(async move {
                    let client = reqwest::Client::builder()
                        .timeout(Duration::from_secs(300))
                        .user_agent(concat!(
                            "ignite-containers-desktop/",
                            env!("CARGO_PKG_VERSION")
                        ))
                        .build()
                        .expect("failed to build HTTP client");

                    while let Some(request) = rx.recv().await {
                        let client = client.clone();
                        let out_tx = out_tx.clone();
                        let ctx = ctx.clone();
                        tokio::spawn(async move {
                            let tag = request.tag.clone();
                            let outcome = execute(&client, request).await;
                            let _ = out_tx.send(ApiResult { tag, outcome });
                            ctx.request_repaint();
                        });
                    }
                });
            })
            .expect("failed to spawn API worker thread");

        Self { tx, rx: out_rx }
    }

    pub fn send(&self, request: ApiRequest) {
        let _ = self.tx.send(request);
    }

    /// Non-blocking drain of everything that finished since the last frame.
    #[must_use]
    pub fn drain(&self) -> Vec<ApiResult> {
        self.rx.try_iter().collect()
    }
}

async fn execute(client: &reqwest::Client, request: ApiRequest) -> Result<ApiResponse, String> {
    let ApiRequest {
        tag: _tag,
        conn,
        method,
        path,
        body,
        download,
    } = request;

    let method = reqwest::Method::from_bytes(method.trim().to_uppercase().as_bytes())
        .map_err(|_| "invalid HTTP method".to_string())?;
    let url = conn.endpoint(&path);
    let mut builder = client
        .request(method, url)
        .bearer_auth(&conn.api_key)
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(body) = &body {
        builder = builder.json(body);
    }

    let response = builder
        .send()
        .await
        .map_err(|error| format!("request failed: {error}"))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);

    if let Some(destination) = download {
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("could not read download: {error}"))?;
        if (200..300).contains(&status) {
            if let Some(parent) = destination.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|error| {
                        format!("could not create {}: {error}", parent.display())
                    })?;
                }
            }
            std::fs::write(&destination, &bytes)
                .map_err(|error| format!("could not write {}: {error}", destination.display()))?;
            return Ok(ApiResponse {
                status,
                content_type,
                body: format!("saved {} bytes to {}", bytes.len(), destination.display()),
            });
        }
        return Ok(ApiResponse {
            status,
            content_type,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }

    let body = response
        .text()
        .await
        .map_err(|error| format!("could not read response: {error}"))?;
    Ok(ApiResponse {
        status,
        content_type,
        body,
    })
}

// --------------------------------------------------------------------- console

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleMode {
    Poll,
    Stream,
}

pub enum ConsoleCommand {
    Watch {
        server_id: String,
        conn: Connection,
        mode: ConsoleMode,
    },
    Pause,
}

pub enum ConsoleEvent {
    Lines {
        server_id: String,
        lines: Vec<String>,
        cursor: String,
        truncated: bool,
    },
    Error {
        server_id: String,
        message: String,
    },
}

pub struct ConsoleWorker {
    tx: tokio::sync::mpsc::UnboundedSender<ConsoleCommand>,
    rx: mpsc::Receiver<ConsoleEvent>,
}

impl ConsoleWorker {
    pub fn new(ctx: Context) -> Self {
        let (tx, mut cmd_rx) = tokio::sync::mpsc::unbounded_channel::<ConsoleCommand>();
        let (out_tx, out_rx) = mpsc::channel::<ConsoleEvent>();

        std::thread::Builder::new()
            .name("ignite-console".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to start console runtime");
                let poll_client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(12))
                    .user_agent(concat!(
                        "ignite-containers-desktop/",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .build()
                    .expect("failed to build console HTTP client");
                let stream_client = reqwest::Client::builder()
                    .user_agent(concat!(
                        "ignite-containers-desktop/",
                        env!("CARGO_PKG_VERSION")
                    ))
                    .build()
                    .expect("failed to build console stream client");

                runtime.block_on(async move {
                    let mut target: Option<(String, Connection, ConsoleMode)> = None;

                    loop {
                        if target.is_none() {
                            match cmd_rx.recv().await {
                                Some(command) => apply_command(command, &mut target),
                                None => return,
                            }
                            continue;
                        }

                        let (server_id, conn, mode) = target.clone().expect("target checked");
                        match mode {
                            ConsoleMode::Poll => {
                                let outcome =
                                    fetch_logs(&poll_client, &conn, &server_id).await;
                                let event = match outcome {
                                    Ok((lines, cursor, truncated)) => ConsoleEvent::Lines {
                                        server_id,
                                        lines,
                                        cursor,
                                        truncated,
                                    },
                                    Err(message) => ConsoleEvent::Error { server_id, message },
                                };
                                let _ = out_tx.send(event);
                                ctx.request_repaint();
                                if !wait_for_command(
                                    &mut cmd_rx,
                                    &mut target,
                                    Duration::from_millis(1500),
                                )
                                .await
                                {
                                    return;
                                }
                            }
                            ConsoleMode::Stream => {
                                let mut interrupted = false;
                                match open_log_stream(&stream_client, &conn, &server_id).await {
                                    Ok(mut response) => {
                                        let mut buffer = String::new();
                                        loop {
                                            tokio::select! {
                                                command = cmd_rx.recv() => {
                                                    match command {
                                                        Some(command) => apply_command(command, &mut target),
                                                        None => return,
                                                    }
                                                    interrupted = true;
                                                    break;
                                                }
                                                chunk = response.chunk() => {
                                                    match chunk {
                                                        Ok(Some(bytes)) => {
                                                            for (kind, data) in parse_sse_chunk(&mut buffer, &bytes) {
                                                                let event = if kind == "error" {
                                                                    ConsoleEvent::Error {
                                                                        server_id: server_id.clone(),
                                                                        message: data,
                                                                    }
                                                                } else {
                                                                    ConsoleEvent::Lines {
                                                                        server_id: server_id.clone(),
                                                                        lines: data.lines().map(str::to_string).collect(),
                                                                        cursor: String::new(),
                                                                        truncated: false,
                                                                    }
                                                                };
                                                                let _ = out_tx.send(event);
                                                            }
                                                            ctx.request_repaint();
                                                        }
                                                        Ok(None) => {
                                                            let _ = out_tx.send(ConsoleEvent::Error {
                                                                server_id: server_id.clone(),
                                                                message: "log stream closed by the server".to_string(),
                                                            });
                                                            break;
                                                        }
                                                        Err(error) => {
                                                            let _ = out_tx.send(ConsoleEvent::Error {
                                                                server_id: server_id.clone(),
                                                                message: format!("log stream error: {error}"),
                                                            });
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(message) => {
                                        let _ = out_tx.send(ConsoleEvent::Error {
                                            server_id: server_id.clone(),
                                            message,
                                        });
                                    }
                                }
                                ctx.request_repaint();
                                if !interrupted
                                    && !wait_for_command(
                                        &mut cmd_rx,
                                        &mut target,
                                        Duration::from_secs(2),
                                    )
                                    .await
                                {
                                    return;
                                }
                            }
                        }
                    }
                });
            })
            .expect("failed to spawn console worker thread");

        Self { tx, rx: out_rx }
    }

    pub fn watch(&self, server_id: &str, conn: &Connection, mode: ConsoleMode) {
        let _ = self.tx.send(ConsoleCommand::Watch {
            server_id: server_id.to_string(),
            conn: conn.clone(),
            mode,
        });
    }

    pub fn pause(&self) {
        let _ = self.tx.send(ConsoleCommand::Pause);
    }

    #[must_use]
    pub fn drain(&self) -> Vec<ConsoleEvent> {
        self.rx.try_iter().collect()
    }
}

fn apply_command(command: ConsoleCommand, target: &mut Option<(String, Connection, ConsoleMode)>) {
    match command {
        ConsoleCommand::Watch {
            server_id,
            conn,
            mode,
        } => *target = Some((server_id, conn, mode)),
        ConsoleCommand::Pause => *target = None,
    }
}

async fn wait_for_command(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<ConsoleCommand>,
    target: &mut Option<(String, Connection, ConsoleMode)>,
    duration: Duration,
) -> bool {
    tokio::select! {
        () = tokio::time::sleep(duration) => true,
        command = rx.recv() => match command {
            Some(command) => {
                apply_command(command, target);
                true
            }
            None => false,
        },
    }
}

async fn open_log_stream(
    client: &reqwest::Client,
    conn: &Connection,
    server_id: &str,
) -> Result<reqwest::Response, String> {
    let response = client
        .get(conn.endpoint(&format!("/api/v1/servers/{server_id}/logs/stream")))
        .bearer_auth(&conn.api_key)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .send()
        .await
        .map_err(|error| format!("log stream failed: {error}"))?;
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        Ok(response)
    } else {
        let text = response.text().await.unwrap_or_default();
        Err(api_error_message(status, &text))
    }
}

fn parse_sse_chunk(buffer: &mut String, bytes: &[u8]) -> Vec<(String, String)> {
    buffer.push_str(&String::from_utf8_lossy(bytes).replace("\r\n", "\n"));
    let mut events = Vec::new();
    while let Some(position) = buffer.find("\n\n") {
        let event = buffer[..position].to_string();
        buffer.drain(..position + 2);
        let mut kind = "log".to_string();
        let mut data = Vec::new();
        for line in event.lines() {
            if let Some(value) = line.strip_prefix("event:") {
                kind = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value).to_string());
            }
        }
        if !data.is_empty() {
            events.push((kind, data.join("\n")));
        }
    }
    events
}

async fn fetch_logs(
    client: &reqwest::Client,
    conn: &Connection,
    server_id: &str,
) -> Result<(Vec<String>, String, bool), String> {
    let response = client
        .get(conn.endpoint(&format!("/api/v1/servers/{server_id}/logs?lines=500")))
        .bearer_auth(&conn.api_key)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| format!("console poll failed: {error}"))?;
    let status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|error| format!("could not read console response: {error}"))?;
    if !(200..300).contains(&status) {
        return Err(api_error_message(status, &text));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("invalid console response: {error}"))?;
    let cursor = value
        .get("cursor")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let truncated = value
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let lines = value
        .get("lines")
        .and_then(Value::as_array)
        .map(|lines| {
            lines
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok((lines, cursor, truncated))
}

/// Length of the largest suffix of `existing` that equals a prefix of `incoming`.
#[must_use]
pub fn tail_overlap(existing: &[String], incoming: &[String]) -> usize {
    let max_overlap = existing.len().min(incoming.len()).min(1000);
    for candidate in (1..=max_overlap).rev() {
        if existing[existing.len() - candidate..] == incoming[..candidate] {
            return candidate;
        }
    }
    0
}

/// Append a tail chunk without duplicating the overlap with the existing view.
///
/// The logs endpoint returns the last N lines, not an increment. We locate the
/// largest suffix of the existing buffer that equals a prefix of the new chunk
/// and append only the remainder. This also handles the common case where a few
/// lines rolled out of the tail between two polls.
pub fn merge_console_lines(existing: &mut Vec<String>, incoming: &[String], cap: usize) -> usize {
    if incoming.is_empty() {
        return 0;
    }
    let overlap = tail_overlap(existing, incoming);
    let new_lines = incoming.len() - overlap;
    existing.extend(incoming[overlap..].iter().cloned());
    let excess = existing.len().saturating_sub(cap);
    if excess > 0 {
        existing.drain(0..excess);
    }
    new_lines
}

#[cfg(test)]
mod tests {
    use super::{
        merge_console_lines, parse_sse_chunk, Connection, ConsoleEvent, ConsoleMode, ConsoleWorker,
    };
    use std::time::{Duration, Instant};

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn first_chunk_is_appended() {
        let mut existing = Vec::new();
        let appended = merge_console_lines(&mut existing, &lines(&["a", "b"]), 10);
        assert_eq!(appended, 2);
        assert_eq!(existing, lines(&["a", "b"]));
    }

    #[test]
    fn identical_tail_is_not_duplicated() {
        let mut existing = lines(&["a", "b", "c"]);
        let appended = merge_console_lines(&mut existing, &lines(&["a", "b", "c"]), 10);
        assert_eq!(appended, 0);
        assert_eq!(existing, lines(&["a", "b", "c"]));
    }

    #[test]
    fn shifted_tail_only_appends_new_lines() {
        let mut existing = lines(&["a", "b", "c", "d"]);
        let appended = merge_console_lines(&mut existing, &lines(&["c", "d", "e"]), 10);
        assert_eq!(appended, 1);
        assert_eq!(existing, lines(&["a", "b", "c", "d", "e"]));
    }

    #[test]
    fn sse_log_events_are_parsed() {
        let mut buffer = String::new();
        let events = parse_sse_chunk(
            &mut buffer,
            b"event: log\ndata: [12:00:00] starting\n\ndata: [12:00:01] ready\n\n",
        );
        assert_eq!(
            events,
            vec![
                ("log".to_string(), "[12:00:00] starting".to_string()),
                ("log".to_string(), "[12:00:01] ready".to_string()),
            ]
        );
    }

    #[test]
    fn sse_keepalive_comments_are_ignored() {
        let mut buffer = String::new();
        let events = parse_sse_chunk(&mut buffer, b": keep-alive\n\ndata: one line\n\n");
        assert_eq!(events, vec![("log".to_string(), "one line".to_string())]);
    }

    #[test]
    fn sse_events_can_span_chunks() {
        let mut buffer = String::new();
        assert!(parse_sse_chunk(&mut buffer, b"event: log\ndata: par").is_empty());
        let events = parse_sse_chunk(&mut buffer, b"tial\n\n");
        assert_eq!(events, vec![("log".to_string(), "partial".to_string())]);
    }

    #[test]
    fn sse_error_events_keep_their_kind() {
        let mut buffer = String::new();
        let events = parse_sse_chunk(&mut buffer, b"event: error\ndata: node offline\n\n");
        assert_eq!(
            events,
            vec![("error".to_string(), "node offline".to_string())]
        );
    }

    #[test]
    fn console_worker_streams_sse_lines() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test SSE server");
        let address = listener.local_addr().expect("test SSE address");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept SSE client");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                )
                .expect("write SSE headers");
            stream
                .write_all(b"event: log\ndata: hello from sse\n\n")
                .expect("write SSE event");
            let _ = stream.flush();
        });

        let worker = ConsoleWorker::new(egui::Context::default());
        worker.watch(
            "server-id",
            &Connection::new(format!("http://{address}"), "test-key"),
            ConsoleMode::Stream,
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut received = Vec::new();
        while Instant::now() < deadline {
            for event in worker.drain() {
                if let ConsoleEvent::Lines { lines, .. } = event {
                    received.extend(lines);
                }
            }
            if received.iter().any(|line| line == "hello from sse") {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        worker.pause();
        assert!(
            received.iter().any(|line| line == "hello from sse"),
            "worker did not deliver the SSE line: {received:?}"
        );
    }

    #[test]
    fn history_is_capped() {
        let mut existing = lines(&["a", "b", "c"]);
        merge_console_lines(&mut existing, &lines(&["d", "e"]), 4);
        assert_eq!(existing, lines(&["b", "c", "d", "e"]));
    }
}
