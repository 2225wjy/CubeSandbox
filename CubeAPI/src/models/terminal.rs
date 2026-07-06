// SPDX-License-Identifier: Apache-2.0
// Copyright (C) 2026 Tencent. All rights reserved.

//! WebSocket message types for the interactive terminal feature.
//!
//! The browser sends [`TerminalInput`] messages (JSON text frames) to CubeAPI;
//! CubeAPI sends [`TerminalOutput`] messages back.  CubeAPI bridges these to
//! the envd Connect protocol running inside the sandbox micro-VM.

use serde::{Deserialize, Serialize};

// ── Browser → CubeAPI ──────────────────────────────────────────────────────

/// Messages sent from the browser terminal (xterm.js) to the CubeAPI WebSocket
/// handler.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub enum TerminalInput {
    /// User keystroke / paste data (base64-encoded raw stdin bytes).
    #[serde(rename = "input")]
    Input {
        /// Base64-encoded stdin data.
        data: String,
    },

    /// Terminal window resize notification.
    #[serde(rename = "resize")]
    Resize {
        rows: u16,
        cols: u16,
    },

    /// Keepalive ping from the client.
    #[serde(rename = "ping")]
    Ping,
}

// ── CubeAPI → Browser ──────────────────────────────────────────────────────

/// Messages sent from the CubeAPI WebSocket handler back to the browser
/// terminal.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum TerminalOutput {
    /// Process stdout/stderr data (base64-encoded).
    #[serde(rename = "output")]
    Output {
        /// Base64-encoded stdout data.
        data: String,
    },

    /// PTY session successfully started.
    #[serde(rename = "started")]
    Started {
        /// Process ID assigned by envd inside the sandbox.
        pid: u64,
    },

    /// Process exited.
    #[serde(rename = "exit")]
    Exit {
        /// Exit code of the shell process.
        code: i32,
    },

    /// An error occurred (connection failure, envd error, etc.).
    #[serde(rename = "error")]
    Error {
        message: String,
    },

    /// Keepalive pong response.
    #[serde(rename = "pong")]
    Pong,
}

// ── WebSocket query parameters ─────────────────────────────────────────────

/// Query parameters for the `GET /sandboxes/:id/terminal` WebSocket upgrade.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalQuery {
    /// Auth token (session token or API key).  Required because browsers
    /// cannot set custom headers on a WebSocket opening handshake.
    pub token: Option<String>,

    /// Target container name when a sandbox hosts multiple containers.
    pub container: Option<String>,

    /// Initial terminal rows (default 24).
    #[serde(default = "default_rows")]
    pub rows: u16,

    /// Initial terminal cols (default 80).
    #[serde(default = "default_cols")]
    pub cols: u16,
}

fn default_rows() -> u16 {
    24
}

fn default_cols() -> u16 {
    80
}

// ── envd Connect-protocol event types ──────────────────────────────────────

/// Parsed event from the envd `process.Process/Start` streaming response.
#[derive(Debug)]
pub enum EnvdEvent {
    /// Process started — carries the PID.
    Start { pid: u64 },
    /// Data event — stdout or stderr chunk (already decoded from base64).
    Data { stdout: Option<Vec<u8>>, stderr: Option<Vec<u8>> },
    /// Process ended — carries the exit code.
    End { exit_code: i32 },
    /// Error from envd.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_input_message() {
        let msg: TerminalInput =
            serde_json::from_str(r#"{"type":"input","data":"aGVsbG8="}"#).unwrap();
        match msg {
            TerminalInput::Input { data } => assert_eq!(data, "aGVsbG8="),
            _ => panic!("expected Input variant"),
        }
    }

    #[test]
    fn deserialize_resize_message() {
        let msg: TerminalInput =
            serde_json::from_str(r#"{"type":"resize","rows":40,"cols":120}"#).unwrap();
        match msg {
            TerminalInput::Resize { rows, cols } => {
                assert_eq!(rows, 40);
                assert_eq!(cols, 120);
            }
            _ => panic!("expected Resize variant"),
        }
    }

    #[test]
    fn deserialize_ping_message() {
        let msg: TerminalInput = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        assert!(matches!(msg, TerminalInput::Ping));
    }

    #[test]
    fn serialize_output_message() {
        let msg = TerminalOutput::Output {
            data: "d29ybGQ=".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"output""#));
        assert!(json.contains(r#""data":"d29ybGQ=""#));
    }

    #[test]
    fn serialize_started_message() {
        let msg = TerminalOutput::Started { pid: 42 };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"started","pid":42}"#);
    }

    #[test]
    fn serialize_exit_message() {
        let msg = TerminalOutput::Exit { code: 0 };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"exit","code":0}"#);
    }

    #[test]
    fn serialize_error_message() {
        let msg = TerminalOutput::Error {
            message: "connection lost".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"error""#));
    }

    #[test]
    fn deserialize_terminal_query_defaults() {
        let q: TerminalQuery = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(q.rows, 24);
        assert_eq!(q.cols, 80);
        assert!(q.token.is_none());
        assert!(q.container.is_none());
    }

    #[test]
    fn deserialize_terminal_query_with_params() {
        let q: TerminalQuery = serde_json::from_str(
            r#"{"token":"abc","container":"main","rows":40,"cols":120}"#,
        )
        .unwrap();
        assert_eq!(q.token.as_deref(), Some("abc"));
        assert_eq!(q.container.as_deref(), Some("main"));
        assert_eq!(q.rows, 40);
        assert_eq!(q.cols, 120);
    }
}
