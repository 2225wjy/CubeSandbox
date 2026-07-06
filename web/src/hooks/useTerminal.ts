// SPDX-License-Identifier: Apache-2.0
// Copyright (C) 2026 Tencent. All rights reserved.

import { useCallback, useEffect, useRef, useState } from 'react';
import type { TerminalInput, TerminalOutput, TerminalStatus } from '@/types/terminal';

const HEARTBEAT_MS = 30_000;
const MAX_RECONNECT_ATTEMPTS = 3;
const RECONNECT_BASE_DELAY_MS = 1_000;

interface UseTerminalOptions {
  sandboxID: string;
  container?: string;
  rows?: number;
  cols?: number;
  /** Called for every message received from the server. */
  onMessage: (msg: TerminalOutput) => void;
  /** Called when the WebSocket closes or errors out. */
  onClose?: (reason: string) => void;
}

interface UseTerminalReturn {
  status: TerminalStatus;
  error: string | null;
  send: (msg: TerminalInput) => void;
  connect: () => void;
  disconnect: () => void;
}

export function useTerminal(opts: UseTerminalOptions): UseTerminalReturn {
  const { sandboxID, container, rows = 24, cols = 80, onMessage, onClose } = opts;

  const [status, setStatus] = useState<TerminalStatus>('disconnected');
  const [error, setError] = useState<string | null>(null);

  const wsRef = useRef<WebSocket | null>(null);
  const heartbeatRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const reconnectCountRef = useRef(0);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onMessageRef = useRef(onMessage);
  const onCloseRef = useRef(onClose);

  // Keep callbacks fresh without re-creating the WebSocket
  useEffect(() => { onMessageRef.current = onMessage; }, [onMessage]);
  useEffect(() => { onCloseRef.current = onClose; }, [onClose]);

  const cleanup = useCallback(() => {
    if (heartbeatRef.current) {
      clearInterval(heartbeatRef.current);
      heartbeatRef.current = null;
    }
    if (reconnectTimerRef.current) {
      clearTimeout(reconnectTimerRef.current);
      reconnectTimerRef.current = null;
    }
    if (wsRef.current) {
      wsRef.current.onclose = null;
      wsRef.current.onerror = null;
      wsRef.current.onmessage = null;
      if (wsRef.current.readyState === WebSocket.OPEN || wsRef.current.readyState === WebSocket.CONNECTING) {
        wsRef.current.close();
      }
      wsRef.current = null;
    }
  }, []);

  const connect = useCallback(() => {
    cleanup();
    setError(null);
    setStatus('connecting');
    reconnectCountRef.current = 0;

    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const token = localStorage.getItem('cube.session') || localStorage.getItem('cube.apiKey') || '';
    const params = new URLSearchParams();
    if (token) params.set('token', token);
    if (container) params.set('container', container);
    params.set('rows', String(rows));
    params.set('cols', String(cols));

    const url = `${protocol}//${window.location.host}/cubeapi/v1/sandboxes/${encodeURIComponent(sandboxID)}/terminal?${params}`;

    const ws = new WebSocket(url);
    wsRef.current = ws;

    ws.onopen = () => {
      setStatus('connected');
      reconnectCountRef.current = 0;

      // Start heartbeat
      heartbeatRef.current = setInterval(() => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: 'ping' }));
        }
      }, HEARTBEAT_MS);
    };

    ws.onmessage = (event: MessageEvent) => {
      try {
        const msg: TerminalOutput = JSON.parse(event.data);
        onMessageRef.current(msg);
      } catch {
        // ignore malformed messages
      }
    };

    ws.onclose = (event: CloseEvent) => {
      cleanup();
      setStatus('disconnected');
      const reason = event.reason || `code ${event.code}`;
      onCloseRef.current?.(reason);
    };

    ws.onerror = () => {
      const errMsg = 'WebSocket connection error';
      setError(errMsg);
      setStatus('error');
      onCloseRef.current?.(errMsg);
    };
  }, [sandboxID, container, rows, cols, cleanup]);

  const disconnect = useCallback(() => {
    reconnectCountRef.current = MAX_RECONNECT_ATTEMPTS; // prevent reconnect
    cleanup();
    setStatus('disconnected');
  }, [cleanup]);

  const send = useCallback((msg: TerminalInput) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(msg));
    }
  }, []);

  // Cleanup on unmount
  useEffect(() => cleanup, [cleanup]);

  return { status, error, send, connect, disconnect };
}
