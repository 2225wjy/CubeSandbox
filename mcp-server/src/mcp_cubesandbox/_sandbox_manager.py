# Copyright (c) 2026 Tencent Inc.
# SPDX-License-Identifier: Apache-2.0

"""Thread-safe sandbox lifecycle manager.

Tracks sandbox instances created through the MCP server so they can be
looked up by ID across tool calls and cleaned up when the server exits.
"""

from __future__ import annotations

import atexit
import logging
import threading
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from cubesandbox import Sandbox

logger = logging.getLogger(__name__)


class SandboxManager:
    """Registry of active sandbox instances managed by this MCP server."""

    def __init__(self) -> None:
        self._sandboxes: dict[str, Sandbox] = {}
        self._lock = threading.Lock()
        atexit.register(self.cleanup_all)

    # ------------------------------------------------------------------
    # Public API
    # ------------------------------------------------------------------

    def register(self, sandbox: Sandbox) -> None:
        """Track a newly created sandbox."""
        with self._lock:
            self._sandboxes[sandbox.sandbox_id] = sandbox
            logger.info("Registered sandbox %s", sandbox.sandbox_id)

    def get(self, sandbox_id: str) -> Sandbox | None:
        """Look up a sandbox by ID. Returns ``None`` if not tracked."""
        with self._lock:
            return self._sandboxes.get(sandbox_id)

    def remove(self, sandbox_id: str) -> Sandbox | None:
        """Remove a sandbox from tracking (e.g. after kill). Returns the
        removed instance or ``None``."""
        with self._lock:
            sb = self._sandboxes.pop(sandbox_id, None)
            if sb:
                logger.info("Removed sandbox %s", sandbox_id)
            return sb

    def list_ids(self) -> list[str]:
        """Return a snapshot of currently tracked sandbox IDs."""
        with self._lock:
            return list(self._sandboxes.keys())

    def cleanup_all(self) -> None:
        """Kill and remove all tracked sandboxes. Called on server exit."""
        with self._lock:
            ids = list(self._sandboxes.keys())

        if not ids:
            return

        logger.info("Cleaning up %d sandbox(es) on exit …", len(ids))
        for sid in ids:
            try:
                sb = self.remove(sid)
                if sb is not None:
                    sb.kill()
                    logger.info("Killed sandbox %s", sid)
            except Exception:
                logger.warning("Failed to kill sandbox %s during cleanup", sid, exc_info=True)
