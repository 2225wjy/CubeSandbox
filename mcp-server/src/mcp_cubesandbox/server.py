# Copyright (c) 2026 Tencent Inc.
# SPDX-License-Identifier: Apache-2.0

"""CubeSandbox MCP Server.

Exposes CubeSandbox capabilities as MCP tools so that Claude Code (and
any other MCP-compatible client) can create sandboxes, execute code,
run shell commands, and manage files inside hardware-isolated MicroVMs.
"""

from __future__ import annotations

import json
import logging
from typing import Any

from cubesandbox import (
    CubeSandboxError,
    Execution,
    Sandbox,
)
from mcp.server.fastmcp import FastMCP

from ._sandbox_manager import SandboxManager

logger = logging.getLogger(__name__)

# ---------------------------------------------------------------------------
# Server & manager singletons
# ---------------------------------------------------------------------------

mcp = FastMCP(
    "CubeSandbox",
    instructions=(
        "CubeSandbox MCP Server — manage hardware-isolated MicroVM sandboxes "
        "for AI agent code execution. Create a sandbox first, then use its ID "
        "for all subsequent operations (run code, run commands, read/write files). "
        "Always kill sandboxes when done to free resources."
    ),
)

_manager = SandboxManager()


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _get_sandbox(sandbox_id: str) -> Sandbox:
    """Look up a tracked sandbox or raise."""
    sb = _manager.get(sandbox_id)
    if sb is None:
        raise KeyError(
            f"Sandbox '{sandbox_id}' not found in this session. Active: {_manager.list_ids()}"
        )
    return sb


def _execution_to_dict(exec: Execution) -> dict[str, Any]:
    """Convert an Execution result into a JSON-friendly dict."""
    result: dict[str, Any] = {}

    # Main result text
    if exec.text is not None:
        result["result"] = exec.text

    # stdout / stderr
    stdout = exec.logs.stdout if exec.logs else []
    stderr = exec.logs.stderr if exec.logs else []
    if stdout:
        result["stdout"] = "\n".join(stdout)
    if stderr:
        result["stderr"] = "\n".join(stderr)

    # Error
    if exec.error:
        result["error"] = {
            "name": exec.error.name,
            "value": exec.error.value,
            "traceback": exec.error.traceback,
        }

    # Rich results (charts, images, etc.)
    rich_results = []
    for r in exec.results:
        entry: dict[str, Any] = {}
        if r.text is not None:
            entry["text"] = r.text
        if r.png is not None:
            entry["png"] = r.png
        if r.html is not None:
            entry["html"] = r.html
        if r.markdown is not None:
            entry["markdown"] = r.markdown
        if r.json is not None:
            entry["json"] = r.json
        if r.is_main_result:
            entry["is_main_result"] = True
        if entry:
            rich_results.append(entry)
    if rich_results:
        result["results"] = rich_results

    return result


# ---------------------------------------------------------------------------
# Tools — Sandbox Lifecycle
# ---------------------------------------------------------------------------


@mcp.tool()
def sandbox_create(
    template_id: str | None = None,
    timeout: int = 300,
) -> str:
    """Create a new sandbox.

    Args:
        template_id: Template ID to use. Falls back to the CUBE_TEMPLATE_ID
            environment variable when omitted.
        timeout: Sandbox TTL in seconds (default 300).

    Returns:
        JSON with sandbox_id, template_id, and state.
    """
    try:
        sb = Sandbox.create(template=template_id, timeout=timeout)
        _manager.register(sb)
        return json.dumps(
            {
                "sandbox_id": sb.sandbox_id,
                "template_id": sb.template_id,
                "timeout": timeout,
            },
            indent=2,
        )
    except (CubeSandboxError, ValueError) as exc:
        return f"Error: {exc}"


@mcp.tool()
def sandbox_kill(sandbox_id: str) -> str:
    """Destroy a sandbox and free its resources.

    Args:
        sandbox_id: ID of the sandbox to destroy.
    """
    try:
        sb = _manager.remove(sandbox_id)
        if sb is None:
            return f"Error: Sandbox '{sandbox_id}' is not tracked by this session."
        sb.kill()
        return f"Sandbox '{sandbox_id}' destroyed."
    except CubeSandboxError as exc:
        return f"Error: {exc}"


@mcp.tool()
def sandbox_list() -> str:
    """List all sandboxes tracked by this session.

    Returns JSON array with sandbox_id and basic info for each.
    """
    ids = _manager.list_ids()
    results = []
    for sid in ids:
        sb = _manager.get(sid)
        if sb is None:
            continue
        try:
            info = sb.get_info()
            results.append(
                {
                    "sandbox_id": sid,
                    "template_id": sb.template_id,
                    "state": info.get("state", "unknown"),
                }
            )
        except Exception:
            results.append(
                {
                    "sandbox_id": sid,
                    "template_id": sb.template_id,
                    "state": "unreachable",
                }
            )
    return json.dumps(results, indent=2)


@mcp.tool()
def sandbox_info(sandbox_id: str) -> str:
    """Get detailed status and metadata for a sandbox.

    Args:
        sandbox_id: ID of the sandbox to query.

    Returns:
        JSON with sandbox state, CPU, memory, and other metadata.
    """
    try:
        sb = _get_sandbox(sandbox_id)
        info = sb.get_info()
        return json.dumps(info, indent=2)
    except KeyError as exc:
        return f"Error: {exc}"
    except CubeSandboxError as exc:
        return f"Error: {exc}"


@mcp.tool()
def sandbox_pause(sandbox_id: str) -> str:
    """Pause a sandbox, preserving its memory snapshot.

    The sandbox can be resumed later with sandbox_resume.
    While paused, it consumes zero compute resources.

    Args:
        sandbox_id: ID of the sandbox to pause.
    """
    try:
        sb = _get_sandbox(sandbox_id)
        sb.pause()
        return f"Sandbox '{sandbox_id}' paused. Use sandbox_resume to restore."
    except KeyError as exc:
        return f"Error: {exc}"
    except (CubeSandboxError, TimeoutError) as exc:
        return f"Error: {exc}"


@mcp.tool()
def sandbox_resume(sandbox_id: str) -> str:
    """Resume a previously paused sandbox.

    Args:
        sandbox_id: ID of the sandbox to resume.

    Returns:
        JSON with sandbox_id and updated state.
    """
    try:
        sb = _get_sandbox(sandbox_id)
        # Connect auto-resumes and returns a fresh Sandbox instance
        new_sb = Sandbox.connect(sb.sandbox_id, config=sb._config)
        _manager.remove(sandbox_id)
        _manager.register(new_sb)
        info = new_sb.get_info()
        return json.dumps(
            {
                "sandbox_id": new_sb.sandbox_id,
                "state": info.get("state", "unknown"),
            },
            indent=2,
        )
    except KeyError as exc:
        return f"Error: {exc}"
    except CubeSandboxError as exc:
        return f"Error: {exc}"


# ---------------------------------------------------------------------------
# Tools — Code & Command Execution
# ---------------------------------------------------------------------------


@mcp.tool()
def run_code(
    sandbox_id: str,
    code: str,
    timeout: int = 30,
) -> str:
    """Execute Python code inside a sandbox.

    Variables persist across calls within the same sandbox session.

    Args:
        sandbox_id: ID of the target sandbox.
        code: Python source code to execute.
        timeout: Maximum execution time in seconds (default 30).

    Returns:
        JSON with result, stdout, stderr, and error (if any).
    """
    try:
        sb = _get_sandbox(sandbox_id)
        execution = sb.run_code(code, timeout=timeout)
        return json.dumps(_execution_to_dict(execution), indent=2)
    except KeyError as exc:
        return f"Error: {exc}"
    except (CubeSandboxError, Exception) as exc:
        return f"Error: {exc}"


@mcp.tool()
def run_command(
    sandbox_id: str,
    command: str,
    timeout: int = 30,
) -> str:
    """Execute a shell command inside a sandbox.

    Args:
        sandbox_id: ID of the target sandbox.
        command: Shell command string to execute.
        timeout: Maximum execution time in seconds (default 30).

    Returns:
        JSON with stdout, stderr, and exit_code.
    """
    try:
        sb = _get_sandbox(sandbox_id)
        result = sb.commands.run(command, timeout=timeout)
        return json.dumps(
            {
                "stdout": result.stdout,
                "stderr": result.stderr,
                "exit_code": result.exit_code,
            },
            indent=2,
        )
    except KeyError as exc:
        return f"Error: {exc}"
    except (CubeSandboxError, RuntimeError, Exception) as exc:
        return f"Error: {exc}"


# ---------------------------------------------------------------------------
# Tools — File System
# ---------------------------------------------------------------------------


@mcp.tool()
def read_file(
    sandbox_id: str,
    path: str,
) -> str:
    """Read a file from the sandbox filesystem.

    Args:
        sandbox_id: ID of the target sandbox.
        path: Absolute path to the file inside the sandbox.

    Returns:
        The file content as text.
    """
    try:
        sb = _get_sandbox(sandbox_id)
        return sb.files.read(path)
    except KeyError as exc:
        return f"Error: {exc}"
    except (OSError, CubeSandboxError) as exc:
        return f"Error: {exc}"


@mcp.tool()
def write_file(
    sandbox_id: str,
    path: str,
    content: str,
) -> str:
    """Write content to a file inside the sandbox.

    Creates parent directories if they don't exist. Overwrites existing files.

    Args:
        sandbox_id: ID of the target sandbox.
        path: Absolute path to the file inside the sandbox.
        content: Text content to write.
    """
    try:
        sb = _get_sandbox(sandbox_id)
        sb.files.write(path, content)
        return f"Written {len(content)} bytes to '{path}' in sandbox '{sandbox_id}'."
    except KeyError as exc:
        return f"Error: {exc}"
    except (OSError, CubeSandboxError) as exc:
        return f"Error: {exc}"


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def main() -> None:
    """Start the MCP server on stdio transport."""
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s [%(name)s] %(levelname)s: %(message)s",
    )
    logger.info("Starting CubeSandbox MCP Server …")
    mcp.run()
