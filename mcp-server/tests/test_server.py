# Copyright (c) 2026 Tencent Inc.
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for the CubeSandbox MCP Server.

These tests mock the cubesandbox SDK to verify tool behaviour without
requiring a live CubeSandbox deployment.
"""

from __future__ import annotations

import json
from unittest.mock import MagicMock, patch

from mcp_cubesandbox._sandbox_manager import SandboxManager

# ---------------------------------------------------------------------------
# SandboxManager tests
# ---------------------------------------------------------------------------


class TestSandboxManager:
    def setup_method(self) -> None:
        self.manager = SandboxManager()

    def test_register_and_get(self) -> None:
        sb = MagicMock()
        sb.sandbox_id = "sb-123"
        self.manager.register(sb)
        assert self.manager.get("sb-123") is sb

    def test_get_missing_returns_none(self) -> None:
        assert self.manager.get("nonexistent") is None

    def test_remove(self) -> None:
        sb = MagicMock()
        sb.sandbox_id = "sb-123"
        self.manager.register(sb)
        removed = self.manager.remove("sb-123")
        assert removed is sb
        assert self.manager.get("sb-123") is None

    def test_remove_missing_returns_none(self) -> None:
        assert self.manager.remove("nonexistent") is None

    def test_list_ids(self) -> None:
        for i in range(3):
            sb = MagicMock()
            sb.sandbox_id = f"sb-{i}"
            self.manager.register(sb)
        ids = self.manager.list_ids()
        assert sorted(ids) == ["sb-0", "sb-1", "sb-2"]

    def test_cleanup_all(self) -> None:
        sandboxes = []
        for i in range(3):
            sb = MagicMock()
            sb.sandbox_id = f"sb-{i}"
            self.manager.register(sb)
            sandboxes.append(sb)

        self.manager.cleanup_all()

        assert self.manager.list_ids() == []
        for sb in sandboxes:
            sb.kill.assert_called_once()

    def test_cleanup_handles_kill_failure(self) -> None:
        sb = MagicMock()
        sb.sandbox_id = "sb-fail"
        sb.kill.side_effect = RuntimeError("boom")
        self.manager.register(sb)

        # Should not raise
        self.manager.cleanup_all()
        assert self.manager.list_ids() == []


# ---------------------------------------------------------------------------
# Tool function tests (with mocked SDK)
# ---------------------------------------------------------------------------


class TestToolFunctions:
    """Test individual tool functions by importing and calling them directly."""

    def setup_method(self) -> None:
        """Reset the manager state before each test."""
        from mcp_cubesandbox import server

        self.server = server
        # Replace the global manager with a fresh one
        self.server._manager = SandboxManager()

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_sandbox_create(self, mock_sandbox_cls: MagicMock) -> None:
        mock_sb = MagicMock()
        mock_sb.sandbox_id = "sb-test-001"
        mock_sb.template_id = "tpl-abc"
        mock_sandbox_cls.create.return_value = mock_sb

        result = self.server.sandbox_create(template_id="tpl-abc", timeout=60)
        data = json.loads(result)
        assert data["sandbox_id"] == "sb-test-001"
        assert data["template_id"] == "tpl-abc"
        assert "sb-test-001" in self.server._manager.list_ids()

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_sandbox_create_error(self, mock_sandbox_cls: MagicMock) -> None:
        from cubesandbox import ApiError

        mock_sandbox_cls.create.side_effect = ApiError("template not found", 404)

        result = self.server.sandbox_create(template_id="tpl-bad")
        assert result.startswith("Error:")

    def test_sandbox_kill_not_tracked(self) -> None:
        result = self.server.sandbox_kill("sb-ghost")
        assert "Error" in result

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_sandbox_kill_success(self, mock_sandbox_cls: MagicMock) -> None:
        mock_sb = MagicMock()
        mock_sb.sandbox_id = "sb-kill-me"
        mock_sb.template_id = "tpl-x"
        mock_sandbox_cls.create.return_value = mock_sb
        self.server.sandbox_create(template_id="tpl-x")

        result = self.server.sandbox_kill("sb-kill-me")
        assert "destroyed" in result
        mock_sb.kill.assert_called_once()

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_run_code(self, mock_sandbox_cls: MagicMock) -> None:
        mock_sb = MagicMock()
        mock_sb.sandbox_id = "sb-code"
        mock_sb.template_id = "tpl-x"
        mock_sandbox_cls.create.return_value = mock_sb
        self.server.sandbox_create(template_id="tpl-x")

        # Mock execution result
        mock_exec = MagicMock()
        mock_exec.text = "42"
        mock_exec.logs.stdout = ["hello\n"]
        mock_exec.logs.stderr = []
        mock_exec.error = None
        mock_exec.results = []
        mock_sb.run_code.return_value = mock_exec

        result = self.server.run_code("sb-code", "print('hello')\n42")
        data = json.loads(result)
        assert data["result"] == "42"
        assert "hello" in data["stdout"]

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_run_command(self, mock_sandbox_cls: MagicMock) -> None:
        mock_sb = MagicMock()
        mock_sb.sandbox_id = "sb-cmd"
        mock_sb.template_id = "tpl-x"
        mock_sandbox_cls.create.return_value = mock_sb
        self.server.sandbox_create(template_id="tpl-x")

        mock_result = MagicMock()
        mock_result.stdout = "hello cube\n"
        mock_result.stderr = ""
        mock_result.exit_code = 0
        mock_sb.commands.run.return_value = mock_result

        result = self.server.run_command("sb-cmd", "echo hello cube")
        data = json.loads(result)
        assert data["exit_code"] == 0
        assert "hello cube" in data["stdout"]

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_read_file(self, mock_sandbox_cls: MagicMock) -> None:
        mock_sb = MagicMock()
        mock_sb.sandbox_id = "sb-read"
        mock_sb.template_id = "tpl-x"
        mock_sandbox_cls.create.return_value = mock_sb
        self.server.sandbox_create(template_id="tpl-x")

        mock_sb.files.read.return_value = "file content here"

        result = self.server.read_file("sb-read", "/etc/hosts")
        assert result == "file content here"

    @patch("mcp_cubesandbox.server.Sandbox")
    def test_write_file(self, mock_sandbox_cls: MagicMock) -> None:
        mock_sb = MagicMock()
        mock_sb.sandbox_id = "sb-write"
        mock_sb.template_id = "tpl-x"
        mock_sandbox_cls.create.return_value = mock_sb
        self.server.sandbox_create(template_id="tpl-x")

        result = self.server.write_file("sb-write", "/tmp/test.txt", "hello")
        assert "Written" in result
        mock_sb.files.write.assert_called_once_with("/tmp/test.txt", "hello")

    def test_run_code_unknown_sandbox(self) -> None:
        result = self.server.run_code("sb-ghost", "1+1")
        assert "Error" in result

    def test_sandbox_list_empty(self) -> None:
        result = self.server.sandbox_list()
        assert json.loads(result) == []
