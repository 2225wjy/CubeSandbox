// SPDX-License-Identifier: Apache-2.0
// Copyright (C) 2026 Tencent. All rights reserved.

import { useCallback, useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Terminal as XTerm } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import { WebLinksAddon } from '@xterm/addon-web-links';
import { useTerminal } from '@/hooks/useTerminal';
import type { TerminalOutput } from '@/types/terminal';
import { Badge } from '@/components/ui/badge';
import { cn } from '@/lib/utils';
import '@xterm/xterm/css/xterm.css';

interface TerminalProps {
  sandboxID: string;
  container?: string;
  className?: string;
}

export function Terminal({ sandboxID, container, className }: TerminalProps) {
  const { t } = useTranslation('sandboxDetail');
  const containerRef = useRef<HTMLDivElement>(null);
  const xtermRef = useRef<XTerm | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);

  // Decode base64 → Uint8Array
  const b64decode = (b64: string): Uint8Array => {
    const bin = atob(b64);
    const bytes = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
    return bytes;
  };

  // Encode Uint8Array → base64
  const b64encode = (bytes: Uint8Array): string => {
    let bin = '';
    for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
    return btoa(bin);
  };

  const onMessage = useCallback(
    (msg: TerminalOutput) => {
      const xterm = xtermRef.current;
      if (!xterm) return;

      switch (msg.type) {
        case 'output':
          xterm.write(b64decode(msg.data));
          break;
        case 'started':
          xterm.writeln(`\r\n\x1b[32m[Connected — PID ${msg.pid}]\x1b[0m\r\n`);
          break;
        case 'exit':
          xterm.writeln(`\r\n\x1b[33m[Process exited with code ${msg.code}]\x1b[0m`);
          break;
        case 'error':
          xterm.writeln(`\r\n\x1b[31m[Error: ${msg.message}]\x1b[0m`);
          break;
        case 'pong':
          break;
      }
    },
    [],
  );

  const { status, error, send, connect, disconnect } = useTerminal({
    sandboxID,
    container,
    onMessage,
    onClose: () => {
      xtermRef.current?.writeln(`\r\n\x1b[31m[${t('terminal.disconnected')}]\x1b[0m`);
    },
  });

  // Initialize xterm
  useEffect(() => {
    if (!containerRef.current) return;

    const xterm = new XTerm({
      cursorBlink: true,
      cursorStyle: 'block',
      fontFamily: '"JetBrains Mono Variable", "JetBrains Mono", monospace',
      fontSize: 13,
      lineHeight: 1.3,
      scrollback: 5000,
      theme: {
        background: '#1e1e2e',
        foreground: '#cdd6f4',
        cursor: '#f5e0dc',
        selectionBackground: '#585b7066',
        black: '#45475a',
        red: '#f38ba8',
        green: '#a6e3a1',
        yellow: '#f9e2af',
        blue: '#89b4fa',
        magenta: '#f5c2e7',
        cyan: '#94e2d5',
        white: '#bac2de',
        brightBlack: '#585b70',
        brightRed: '#f38ba8',
        brightGreen: '#a6e3a1',
        brightYellow: '#f9e2af',
        brightBlue: '#89b4fa',
        brightMagenta: '#f5c2e7',
        brightCyan: '#94e2d5',
        brightWhite: '#a6adc8',
      },
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();

    xterm.loadAddon(fitAddon);
    xterm.loadAddon(webLinksAddon);

    xterm.open(containerRef.current);

    // Fit to container after a brief delay for DOM layout
    requestAnimationFrame(() => {
      try { fitAddon.fit(); } catch { /* ignore */ }
    });

    xtermRef.current = xterm;
    fitAddonRef.current = fitAddon;

    // Connect to WebSocket
    connect();

    // Send keystrokes to WebSocket
    xterm.onData((data: string) => {
      const encoded = b64encode(new TextEncoder().encode(data));
      send({ type: 'input', data: encoded });
    });

    // Handle terminal resize
    xterm.onResize(({ rows, cols }) => {
      send({ type: 'resize', rows, cols });
    });

    // Observe container resize → refit terminal
    const resizeObserver = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        try { fitAddon.fit(); } catch { /* ignore */ }
      });
    });
    if (containerRef.current) {
      resizeObserver.observe(containerRef.current);
    }

    return () => {
      resizeObserver.disconnect();
      disconnect();
      xterm.dispose();
      xtermRef.current = null;
      fitAddonRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sandboxID, container]);

  const statusTone =
    status === 'connected' ? 'ok' :
    status === 'connecting' ? 'warn' :
    status === 'error' ? 'err' : 'mute';

  return (
    <div className={cn('relative', className)}>
      {/* Status bar */}
      <div className="mb-2 flex items-center gap-2 text-xs">
        <Badge tone={statusTone as any}>
          {status === 'connected' ? 'Connected' :
           status === 'connecting' ? t('terminal.connecting') :
           status === 'error' ? 'Error' :
           t('terminal.disconnected')}
        </Badge>
        {error && <span className="text-cube-err/80">{error}</span>}
        <div className="flex-1" />
        {status === 'disconnected' && (
          <button
            onClick={connect}
            className="text-xs text-blue-500 hover:underline"
          >
            {t('terminal.reconnect')}
          </button>
        )}
      </div>

      {/* Terminal container */}
      <div
        ref={containerRef}
        className="min-h-[300px] rounded-md ring-1 ring-border/60"
        style={{ height: '400px' }}
      />
    </div>
  );
}
