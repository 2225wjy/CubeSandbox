// SPDX-License-Identifier: Apache-2.0
// Copyright (C) 2026 Tencent. All rights reserved.

//! WebSocket handler for interactive terminal sessions.
//!
//! Bridges the browser (xterm.js over WebSocket) to the envd agent running
//! inside the sandbox micro-VM (Connect protocol over HTTP).
//!
//! ## Architecture
//!
//! A **single** long-lived HTTP POST to envd's `process.Process/Start`
//! endpoint returns a streaming body.  We read Connect binary-envelope frames
//! from that stream and forward stdout data to the browser.  Keystrokes and
//! resize events from the browser are dispatched as **separate** unary HTTP
//! POSTs to `process.Process/SendInput` and `process.Process/Update`.
//!
//! ```text
//! Browser (xterm.js)                CubeAPI                    Sandbox (envd)
//! ──────────────────  WS text frame  ──────  HTTP POST  ─────────────────────
//! { type: "input" }  ──────────────► SendInput (unary)  ───► stdin
//! { type: "resize" } ──────────────► Update (unary)     ───► TtyWinResize
//! { type: "ping" }   ──► pong
//!                    ◄── output_tx ◄── bytes_stream() ◄─── Start (streaming)
//!                    ◄── { type: "output", data: "…" }
//! ```

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::Response,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::{
    error::AppError,
    models::terminal::{EnvdEvent, TerminalInput, TerminalOutput},
    state::AppState,
};

// ── Constants ──────────────────────────────────────────────────────────────

const ENVD_PORT: u16 = 49983;
const CONNECT_JSON: &str = "application/connect+json";
/// envd basic-auth header (base64 of "root:").
const ENVD_AUTH: &str = "Basic cm9vdDo=";

/// Idle timeout: close session after 30 minutes without browser activity.
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Heartbeat interval.
const HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

// ── Query parameters ───────────────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalQuery {
    pub token: Option<String>,
    pub container: Option<String>,
    #[serde(default = "default_rows")]
    pub rows: u16,
    #[serde(default = "default_cols")]
    pub cols: u16,
}

fn default_rows() -> u16 {
    24
}
fn default_cols() -> u16 {
    80
}

// ── Public handler ─────────────────────────────────────────────────────────

/// `GET /sandboxes/:sandboxID/terminal` — WebSocket upgrade.
pub async fn terminal_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Path(sandbox_id): Path<String>,
    Query(params): Query<TerminalQuery>,
) -> Result<Response, AppError> {
    // ── 1. Auth ────────────────────────────────────────────────────────────
    if let Some(ref callback_url) = state.config.auth_callback_url {
        if !callback_url.is_empty() {
            let token = params.token.as_deref().unwrap_or("");
            if token.is_empty() {
                return Err(AppError::Unauthorized(
                    "Missing ?token= query parameter".into(),
                ));
            }
            validate_ws_token(&state, callback_url, token, &sandbox_id).await?;
        }
    }

    // ── 2. Verify sandbox is running ───────────────────────────────────────
    let detail = state.services.sandboxes.get_sandbox(&sandbox_id).await?;
    if detail.state != crate::models::SandboxState::Running {
        return Err(AppError::BadRequest(format!(
            "Sandbox {} is {:?}, terminal requires Running",
            sandbox_id, detail.state,
        )));
    }

    let domain = detail
        .domain
        .unwrap_or_else(|| state.config.sandbox_domain.clone());

    tracing::info!(
        sandbox_id = %sandbox_id,
        domain = %domain,
        container = ?params.container,
        rows = params.rows,
        cols = params.cols,
        "terminal session opened",
    );

    // ── 3. Upgrade ─────────────────────────────────────────────────────────
    Ok(ws.on_upgrade(move |socket| {
        run_session(socket, state, sandbox_id, domain, params)
    }))
}

// ── Auth helper ────────────────────────────────────────────────────────────

async fn validate_ws_token(
    state: &AppState,
    callback_url: &str,
    token: &str,
    sandbox_id: &str,
) -> Result<(), AppError> {
    let resp = state
        .http_client
        .post(callback_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("X-Request-Path", format!("/sandboxes/{}/terminal", sandbox_id))
        .header("X-Request-Method", "GET")
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("auth callback failed: {}", e)))?;

    if resp.status().as_u16() == 200 {
        Ok(())
    } else {
        Err(AppError::Unauthorized(
            "Authentication rejected for terminal session".into(),
        ))
    }
}

// ── Session loop ───────────────────────────────────────────────────────────

async fn run_session(
    socket: WebSocket,
    state: AppState,
    sandbox_id: String,
    domain: String,
    params: TerminalQuery,
) {
    let (mut ws_tx, mut ws_rx) = socket.split();

    // ── Start shell — returns (pid, streaming body reader) ─────────────────
    let start = open_envd_stream(
        &state,
        &sandbox_id,
        &domain,
        params.container.as_deref(),
        params.rows,
        params.cols,
    )
    .await;

    let (pid, mut envd_chunks) = match start {
        Ok(v) => v,
        Err(e) => {
            send_ws(&mut ws_tx, &TerminalOutput::Error {
                message: format!("Failed to start shell: {}", e),
            })
            .await;
            tracing::warn!(sandbox_id = %sandbox_id, error = %e, "shell start failed");
            return;
        }
    };

    send_ws(&mut ws_tx, &TerminalOutput::Started { pid }).await;

    // ── Input channel: WS → envd SendInput / Update ────────────────────────
    let (input_tx, mut input_rx) = mpsc::channel::<TerminalInput>(256);

    let envd_writer = tokio::spawn({
        let state = state.clone();
        let sandbox_id = sandbox_id.clone();
        let domain = domain.clone();
        let container = params.container.clone();
        async move {
            while let Some(msg) = input_rx.recv().await {
                let r = match msg {
                    TerminalInput::Input { data } => {
                        send_envd_input(&state, &sandbox_id, &domain, pid, &data, container.as_deref()).await
                    }
                    TerminalInput::Resize { rows, cols } => {
                        send_envd_resize(&state, &sandbox_id, &domain, pid, rows, cols, container.as_deref()).await
                    }
                    TerminalInput::Ping => Ok(()),
                };
                if let Err(e) = r {
                    tracing::warn!(error = %e, "envd write failed");
                }
            }
        }
    });

    // ── Main select loop ───────────────────────────────────────────────────
    let mut frame_buf: Vec<u8> = Vec::with_capacity(8192);
    let mut idle_timer = tokio::time::interval(IDLE_TIMEOUT);
    idle_timer.tick().await; // consume first immediate tick
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);

    loop {
        tokio::select! {
            // ── envd streaming output ──
            chunk = envd_chunks.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        frame_buf.extend_from_slice(&bytes);
                        // Parse complete frames from the buffer
                        let events = drain_connect_frames(&mut frame_buf);
                        for event in events {
                            match event {
                                EnvdEvent::Start { .. } => {
                                    // Already handled above; ignore duplicates
                                }
                                EnvdEvent::Data { stdout, stderr } => {
                                    let mut combined = Vec::new();
                                    if let Some(d) = stdout { combined.extend_from_slice(&d); }
                                    if let Some(d) = stderr { combined.extend_from_slice(&d); }
                                    if !combined.is_empty() {
                                        let msg = TerminalOutput::Output {
                                            data: BASE64.encode(&combined),
                                        };
                                        if send_ws(&mut ws_tx, &msg).await.is_err() { break; }
                                    }
                                }
                                EnvdEvent::End { exit_code } => {
                                    send_ws(&mut ws_tx, &TerminalOutput::Exit { code: exit_code }).await;
                                    // Process ended — exit loop
                                    envd_writer.abort();
                                    tracing::info!(sandbox_id = %sandbox_id, pid, exit_code, "shell exited");
                                    return;
                                }
                                EnvdEvent::Error(msg) => {
                                    send_ws(&mut ws_tx, &TerminalOutput::Error { message: msg }).await;
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, sandbox_id = %sandbox_id, "envd stream error");
                        send_ws(&mut ws_tx, &TerminalOutput::Error {
                            message: format!("Stream error: {}", e),
                        }).await;
                        break;
                    }
                    None => {
                        // Stream ended (connection closed)
                        tracing::info!(sandbox_id = %sandbox_id, "envd stream closed");
                        send_ws(&mut ws_tx, &TerminalOutput::Exit { code: -1 }).await;
                        break;
                    }
                }
            }

            // ── Browser input ──
            msg = ws_rx.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        idle_timer.reset();
                        match serde_json::from_str::<TerminalInput>(&text) {
                            Ok(TerminalInput::Ping) => {
                                send_ws(&mut ws_tx, &TerminalOutput::Pong).await;
                            }
                            Ok(input) => { let _ = input_tx.send(input).await; }
                            Err(e) => {
                                tracing::debug!(error = %e, "invalid terminal WS message");
                            }
                        }
                    }
                    Some(Ok(Message::Binary(data))) => {
                        idle_timer.reset();
                        let encoded = BASE64.encode(&data);
                        let _ = input_tx.send(TerminalInput::Input { data: encoded }).await;
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        tracing::info!(sandbox_id = %sandbox_id, "WS closed by client");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(error = %e, "WS read error");
                        break;
                    }
                    _ => {}
                }
            }

            // ── Heartbeat ──
            _ = heartbeat.tick() => {
                if send_ws(&mut ws_tx, &TerminalOutput::Pong).await.is_err() { break; }
            }

            // ── Idle timeout ──
            _ = idle_timer.tick() => {
                tracing::info!(sandbox_id = %sandbox_id, "idle timeout");
                send_ws(&mut ws_tx, &TerminalOutput::Error {
                    message: "Session timed out due to inactivity".into(),
                }).await;
                break;
            }
        }
    }

    // ── Cleanup ────────────────────────────────────────────────────────────
    envd_writer.abort();
    let _ = send_envd_signal(&state, &sandbox_id, &domain, pid, params.container.as_deref()).await;
    tracing::info!(sandbox_id = %sandbox_id, pid, "terminal session closed");
}

// ── envd streaming start ───────────────────────────────────────────────────

/// Open a streaming connection to envd's `process.Process/Start` and extract
/// the PID from the first frame.  Returns `(pid, byte-stream)` where the
/// stream yields raw response-body chunks that still need Connect-frame parsing.
async fn open_envd_stream(
    state: &AppState,
    sandbox_id: &str,
    domain: &str,
    _container: Option<&str>,
    rows: u16,
    cols: u16,
) -> Result<(u64, impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>>), AppError> {
    let host = format!("{}-{}.{}", ENVD_PORT, sandbox_id, domain);
    let proxy_url = std::env::var("AGENTHUB_SANDBOX_PROXY_URL")
        .unwrap_or_else(|_| "http://127.0.0.1".into());
    let url = format!("{}/process.Process/Start", proxy_url.trim_end_matches('/'));

    let req_body = json!({
        "process": {
            "cmd": "/bin/bash",
            "args": ["-i", "-l"],
            "envs": { "TERM": "xterm-256color", "LANG": "en_US.UTF-8" }
        },
        "pty": { "size": { "rows": rows as u32, "cols": cols as u32 } }
    });

    let body = connect_envelope(&serde_json::to_vec(&req_body).unwrap());

    let resp = state
        .http_client
        .post(&url)
        .header("Host", &host)
        .header("Content-Type", CONNECT_JSON)
        .header("Authorization", ENVD_AUTH)
        .body(body)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("envd Start failed: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(AppError::Internal(anyhow::anyhow!(
            "envd Start HTTP {}: {}", status, text
        )));
    }

    // We need to read the *first* Connect frame to get the PID before
    // returning the stream.  Accumulate bytes until we have a complete frame.
    let mut stream = resp.bytes_stream();
    let mut buf = Vec::with_capacity(256);

    // Read until we have the start event
    let mut pid: Option<u64> = None;
    while pid.is_none() {
        match stream.next().await {
            Some(Ok(chunk)) => {
                buf.extend_from_slice(&chunk);
                // Try to parse a start frame from the buffer
                if let Some((events, consumed)) = try_parse_connect_frames(&buf) {
                    for event in &events {
                        if let EnvdEvent::Start { pid: p } = event {
                            pid = Some(*p);
                        }
                    }
                    // Remove consumed bytes, keep remainder for the stream
                    buf.drain(..consumed);
                }
            }
            Some(Err(e)) => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "envd stream read error during start: {}", e
                )));
            }
            None => {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "envd stream closed before start event"
                )));
            }
        }
    }

    let pid = pid.unwrap();

    // Prepend any leftover bytes to the stream
    let leftover = buf;
    let combined = futures::stream::once(async move {
        if leftover.is_empty() {
            None
        } else {
            Some(Ok::<bytes::Bytes, reqwest::Error>(bytes::Bytes::from(leftover)))
        }
    })
    .filter_map(|x| async move { x })
    .chain(stream);

    Ok((pid, combined))
}

// ── envd unary calls ───────────────────────────────────────────────────────

async fn send_envd_input(
    state: &AppState,
    sandbox_id: &str,
    domain: &str,
    pid: u64,
    data_b64: &str,
    _container: Option<&str>,
) -> Result<(), AppError> {
    let host = format!("{}-{}.{}", ENVD_PORT, sandbox_id, domain);
    let url = envd_url("SendInput");
    let body = json!({
        "process": { "pid": pid },
        "input": { "pty": data_b64 }
    });

    let resp = state
        .http_client
        .post(&url)
        .header("Host", &host)
        .header("Content-Type", "application/json")
        .header("Authorization", ENVD_AUTH)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SendInput: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "SendInput HTTP {}", resp.status()
        )));
    }
    Ok(())
}

async fn send_envd_resize(
    state: &AppState,
    sandbox_id: &str,
    domain: &str,
    pid: u64,
    rows: u16,
    cols: u16,
    _container: Option<&str>,
) -> Result<(), AppError> {
    let host = format!("{}-{}.{}", ENVD_PORT, sandbox_id, domain);
    let url = envd_url("Update");
    let body = json!({
        "process": { "pid": pid },
        "pty": { "size": { "rows": rows as u32, "cols": cols as u32 } }
    });

    let resp = state
        .http_client
        .post(&url)
        .header("Host", &host)
        .header("Content-Type", "application/json")
        .header("Authorization", ENVD_AUTH)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Update: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "Update HTTP {}", resp.status()
        )));
    }
    Ok(())
}

async fn send_envd_signal(
    state: &AppState,
    sandbox_id: &str,
    domain: &str,
    pid: u64,
    _container: Option<&str>,
) -> Result<(), AppError> {
    let host = format!("{}-{}.{}", ENVD_PORT, sandbox_id, domain);
    let url = envd_url("SendSignal");
    let body = json!({
        "process": { "pid": pid },
        "signal": "SIGNAL_SIGTERM"
    });

    let resp = state
        .http_client
        .post(&url)
        .header("Host", &host)
        .header("Content-Type", "application/json")
        .header("Authorization", ENVD_AUTH)
        .json(&body)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("SendSignal: {}", e)))?;

    if !resp.status().is_success() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "SendSignal HTTP {}", resp.status()
        )));
    }
    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn envd_url(method: &str) -> String {
    let proxy = std::env::var("AGENTHUB_SANDBOX_PROXY_URL")
        .unwrap_or_else(|_| "http://127.0.0.1".into());
    format!(
        "{}/process.Process/{}",
        proxy.trim_end_matches('/'),
        method
    )
}

async fn send_ws(
    tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &TerminalOutput,
) -> Result<(), ()> {
    let text = serde_json::to_string(msg).unwrap();
    tx.send(Message::Text(text.into())).await.map_err(|_| ())
}

/// Wrap JSON in Connect binary envelope: `[0x00][4B BE len][payload]`.
fn connect_envelope(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    out.push(0);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// Parse and drain complete Connect frames from a byte buffer.
/// Returns the events parsed and the number of bytes consumed.
fn try_parse_connect_frames(buf: &[u8]) -> Option<(Vec<EnvdEvent>, usize)> {
    let mut events = Vec::new();
    let mut i = 0usize;

    while i + 5 <= buf.len() {
        let flags = buf[i];
        let len = u32::from_be_bytes([buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4]]) as usize;

        if i + 5 + len > buf.len() {
            break; // incomplete frame — wait for more data
        }

        let payload = &buf[i + 5..i + 5 + len];
        i += 5 + len;

        if let Some(event) = parse_envd_event(flags, payload) {
            events.push(event);
        }
    }

    if events.is_empty() && i == 0 {
        None
    } else {
        Some((events, i))
    }
}

/// Parse and drain complete Connect frames, consuming the buffer in place.
fn drain_connect_frames(buf: &mut Vec<u8>) -> Vec<EnvdEvent> {
    let mut events = Vec::new();
    let mut i = 0usize;

    while i + 5 <= buf.len() {
        let flags = buf[i];
        let len = u32::from_be_bytes([buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4]]) as usize;

        if i + 5 + len > buf.len() {
            break;
        }

        let payload = &buf[i + 5..i + 5 + len];
        if let Some(event) = parse_envd_event(flags, payload) {
            events.push(event);
        }
        i += 5 + len;
    }

    buf.drain(..i);
    events
}

/// Parse a single envd event from a Connect frame payload.
fn parse_envd_event(flags: u8, payload: &[u8]) -> Option<EnvdEvent> {
    let v: Value = match serde_json::from_slice(payload) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!(error = %e, "invalid envd JSON frame");
            return None;
        }
    };

    // Error frame (flags bit 1)
    if flags & 0b10 != 0 {
        let msg = v
            .get("error")
            .or_else(|| v.get("message"))
            .map(|e| e.to_string())
            .unwrap_or_else(|| "unknown envd error".into());
        return Some(EnvdEvent::Error(msg));
    }

    let event = v.get("event")?;

    if let Some(start) = event.get("start") {
        let pid = start.get("pid").and_then(|p| p.as_u64()).unwrap_or(0);
        return Some(EnvdEvent::Start { pid });
    }

    if let Some(data) = event.get("data") {
        let stdout = data
            .get("stdout")
            .and_then(|s| s.as_str())
            .and_then(|s| BASE64.decode(s).ok());
        let stderr = data
            .get("stderr")
            .and_then(|s| s.as_str())
            .and_then(|s| BASE64.decode(s).ok());
        if stdout.is_some() || stderr.is_some() {
            return Some(EnvdEvent::Data { stdout, stderr });
        }
    }

    if let Some(end) = event.get("end") {
        let exit_code = end
            .get("exitCode")
            .and_then(|c| c.as_i64())
            .or_else(|| {
                end.get("status")
                    .and_then(|s| s.as_str())
                    .and_then(|s| s.strip_prefix("exit status "))
                    .and_then(|n| n.trim().parse::<i64>().ok())
            })
            .unwrap_or(-1) as i32;
        return Some(EnvdEvent::End { exit_code });
    }

    None
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connect_envelope() {
        let frame = connect_envelope(b"hello");
        assert_eq!(frame[0], 0);
        assert_eq!(u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]), 5);
        assert_eq!(&frame[5..], b"hello");
    }

    #[test]
    fn test_parse_start_event() {
        let payload = json!({ "event": { "start": { "pid": 42 } } });
        let frame = connect_envelope(&serde_json::to_vec(&payload).unwrap());
        let mut buf = frame;
        let events = drain_connect_frames(&mut buf);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EnvdEvent::Start { pid: 42 }));
    }

    #[test]
    fn test_parse_data_event() {
        let payload = json!({ "event": { "data": { "stdout": "aGVsbG8=" } } });
        let frame = connect_envelope(&serde_json::to_vec(&payload).unwrap());
        let mut buf = frame;
        let events = drain_connect_frames(&mut buf);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EnvdEvent::Data { stdout, .. } => {
                assert_eq!(stdout.as_deref(), Some(b"hello".as_slice()));
            }
            _ => panic!("expected Data"),
        }
    }

    #[test]
    fn test_parse_end_event() {
        let payload = json!({ "event": { "end": { "exitCode": 0 } } });
        let frame = connect_envelope(&serde_json::to_vec(&payload).unwrap());
        let mut buf = frame;
        let events = drain_connect_frames(&mut buf);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], EnvdEvent::End { exit_code: 0 }));
    }

    #[test]
    fn test_parse_error_frame() {
        let payload = json!({ "error": "permission denied" });
        let json_bytes = serde_json::to_vec(&payload).unwrap();
        let mut frame = vec![0x02u8]; // error flag
        frame.extend_from_slice(&(json_bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(&json_bytes);

        let mut buf = frame;
        let events = drain_connect_frames(&mut buf);
        assert_eq!(events.len(), 1);
        match &events[0] {
            EnvdEvent::Error(msg) => assert!(msg.contains("permission denied")),
            _ => panic!("expected Error"),
        }
    }

    #[test]
    fn test_parse_multiple_frames() {
        let p1 = json!({ "event": { "start": { "pid": 1 } } });
        let p2 = json!({ "event": { "data": { "stdout": "bHM=" } } });
        let p3 = json!({ "event": { "end": { "exitCode": 0 } } });

        let mut stream = Vec::new();
        stream.extend_from_slice(&connect_envelope(&serde_json::to_vec(&p1).unwrap()));
        stream.extend_from_slice(&connect_envelope(&serde_json::to_vec(&p2).unwrap()));
        stream.extend_from_slice(&connect_envelope(&serde_json::to_vec(&p3).unwrap()));

        let events = drain_connect_frames(&mut stream);
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_partial_frame_handling() {
        let payload = json!({ "event": { "start": { "pid": 1 } } });
        let full = connect_envelope(&serde_json::to_vec(&payload).unwrap());

        // Only send half the frame
        let partial = &full[..full.len() / 2];
        let mut buf = partial.to_vec();
        let events = drain_connect_frames(&mut buf);
        assert!(events.is_empty());
        // Buffer should be unchanged (incomplete frame stays)
        assert_eq!(buf.len(), partial.len());
    }

    #[test]
    fn test_exit_status_parsing() {
        let payload = json!({ "event": { "end": { "status": "exit status 137" } } });
        let frame = connect_envelope(&serde_json::to_vec(&payload).unwrap());
        let mut buf = frame;
        let events = drain_connect_frames(&mut buf);
        assert!(matches!(events[0], EnvdEvent::End { exit_code: 137 }));
    }
}
