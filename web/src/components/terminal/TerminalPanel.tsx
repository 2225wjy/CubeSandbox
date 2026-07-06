// SPDX-License-Identifier: Apache-2.0
// Copyright (C) 2026 Tencent. All rights reserved.

import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Maximize2, Minimize2, X } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import { Terminal } from './Terminal';

interface TerminalPanelProps {
  sandboxID: string;
  container?: string;
  onClose: () => void;
}

export function TerminalPanel({ sandboxID, container, onClose }: TerminalPanelProps) {
  const { t } = useTranslation('sandboxDetail');
  const [fullscreen, setFullscreen] = useState(false);

  return (
    <div
      className={cn(
        'flex flex-col',
        fullscreen
          ? 'fixed inset-0 z-50 bg-background p-4'
          : 'relative',
      )}
    >
      {/* Toolbar */}
      <div className="mb-2 flex items-center gap-2">
        <span className="text-sm font-medium text-muted-foreground">
          {t('terminal')}
        </span>
        <div className="flex-1" />
        <Button
          variant="ghost"
          size="icon"
          title={fullscreen ? t('terminal.exitFullscreen') : t('terminal.fullscreen')}
          onClick={() => setFullscreen((v) => !v)}
        >
          {fullscreen ? <Minimize2 size={14} /> : <Maximize2 size={14} />}
        </Button>
        <Button
          variant="ghost"
          size="icon"
          title={t('terminal.hide')}
          onClick={onClose}
        >
          <X size={14} />
        </Button>
      </div>

      {/* Terminal */}
      <Terminal
        sandboxID={sandboxID}
        container={container}
        className={cn(fullscreen && 'flex-1')}
      />
    </div>
  );
}
