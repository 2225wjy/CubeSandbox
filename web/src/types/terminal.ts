// SPDX-License-Identifier: Apache-2.0
// Copyright (C) 2026 Tencent. All rights reserved.

/** Messages sent from browser → CubeAPI WebSocket. */
export type TerminalInput =
  | { type: 'input'; data: string }
  | { type: 'resize'; rows: number; cols: number }
  | { type: 'ping' };

/** Messages sent from CubeAPI WebSocket → browser. */
export type TerminalOutput =
  | { type: 'output'; data: string }
  | { type: 'started'; pid: number }
  | { type: 'exit'; code: number }
  | { type: 'error'; message: string }
  | { type: 'pong' };

/** Connection status for the terminal WebSocket. */
export type TerminalStatus = 'disconnected' | 'connecting' | 'connected' | 'error';
