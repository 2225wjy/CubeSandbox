---
title: Claude Code Integration Guide
author: cubesandbox-team
date: 2026-07-01
tags:
  - integration
  - claude-code
  - mcp
lang: en-US
---

# Claude Code Integration Guide

## Integration Target and Version

[Claude Code](https://docs.anthropic.com/en/docs/claude-code) is Anthropic's AI-powered CLI development assistant. This guide covers integrating Claude Code with CubeSandbox via the **Model Context Protocol (MCP)**, allowing Claude Code to create sandboxes, execute code, run shell commands, and manage files inside hardware-isolated MicroVMs.

| Component | Version |
|-----------|---------|
| Claude Code | Latest (CLI) |
| MCP Server | `mcp-cubesandbox` 0.1.0 |
| CubeSandbox | ≥ 0.3.0 |
| Python | ≥ 3.10 |

## How It Works

```
Claude Code (MCP Client)
    │  stdio (MCP Protocol)
    ▼
mcp-cubesandbox (MCP Server)
    │  cubesandbox Python SDK
    ▼
CubeAPI (:3000) ── Control Plane
    │
CubeMaster → Cubelet → KVM MicroVM (envd)
                           │
CubeProxy ─────────────────┘ Data Plane
```

The MCP Server acts as an adapter: it translates Claude Code's tool calls into CubeSandbox REST API requests. Any MCP-compatible client (not just Claude Code) can use the same server.

## Prerequisites

- A running CubeSandbox deployment with CubeAPI reachable
- A sandbox template created (see [Quick Start](../quickstart.md))
- Python 3.10+ on the machine running Claude Code
- [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code) installed

## Integration Steps

### 1. Install the MCP Server

Clone the CubeSandbox repository and install the MCP server package:

```bash
git clone https://github.com/TencentCloud/CubeSandbox.git
cd CubeSandbox/mcp-server
pip install -e .
```

### 2. Configure Environment Variables

Set the standard CubeSandbox environment variables:

```bash
export CUBE_API_URL="http://<cube-host>:3000"
export CUBE_TEMPLATE_ID="<your-template-id>"

# Optional — for remote access or HTTPS with mkcert
export CUBE_PROXY_NODE_IP="<proxy-node-ip>"
export SSL_CERT_FILE="/root/.local/share/mkcert/rootCA.pem"
```

| Variable | Required | Description |
|----------|:--------:|-------------|
| `CUBE_API_URL` | ✅ | CubeAPI address (port 3000) |
| `CUBE_TEMPLATE_ID` | ✅ | Default template for sandbox creation |
| `E2B_API_KEY` | ❌ | Auth key (any non-empty value for local deploy) |
| `CUBE_PROXY_NODE_IP` | remote | CubeProxy node IP for DNS bypass |
| `SSL_CERT_FILE` | HTTPS | mkcert CA certificate path |

### 3. Configure Claude Code

**Option A: Project-level `.mcp.json`** (recommended)

Create `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "cubesandbox": {
      "command": "python",
      "args": ["-m", "mcp_cubesandbox.server"],
      "env": {
        "CUBE_API_URL": "http://<cube-host>:3000",
        "CUBE_TEMPLATE_ID": "<your-template-id>"
      }
    }
  }
}
```

If using `uv`:

```json
{
  "mcpServers": {
    "cubesandbox": {
      "command": "uv",
      "args": ["run", "--directory", "/path/to/CubeSandbox/mcp-server", "mcp-cubesandbox"],
      "env": {
        "CUBE_API_URL": "http://<cube-host>:3000",
        "CUBE_TEMPLATE_ID": "<your-template-id>"
      }
    }
  }
}
```

**Option B: CLI command**

```bash
claude mcp add \
  --env CUBE_API_URL="http://<cube-host>:3000" \
  --env CUBE_TEMPLATE_ID="<your-template-id>" \
  --transport stdio \
  cubesandbox \
  -- python -m mcp_cubesandbox.server
```

### 4. Verify the Integration

Start Claude Code and check that the MCP server connected:

```bash
claude
```

Inside Claude Code, ask:

```
> List my available sandboxes
```

Claude Code should call the `sandbox_list` tool and return the result.

## Available Tools

| Tool | Description |
|------|-------------|
| `sandbox_create` | Create a new sandbox. Returns `sandbox_id`. |
| `sandbox_kill` | Destroy a sandbox and free resources. |
| `sandbox_list` | List all active sandboxes in this session. |
| `sandbox_info` | Get detailed sandbox status (state, CPU, memory). |
| `sandbox_pause` | Pause a sandbox (preserves memory snapshot). |
| `sandbox_resume` | Resume a paused sandbox. |
| `run_code` | Execute Python code inside a sandbox. Variables persist across calls. |
| `run_command` | Execute a shell command inside a sandbox. |
| `read_file` | Read a file from the sandbox filesystem. |
| `write_file` | Write content to a file inside the sandbox. |

## Usage Examples

### Basic: Create, Run, Destroy

```
> Create a sandbox, then run `print("Hello from CubeSandbox!")` inside it, and destroy it when done.
```

Claude Code will automatically:
1. Call `sandbox_create` → receive a `sandbox_id`
2. Call `run_code` with the ID and Python code
3. Show you the output
4. Call `sandbox_kill` to clean up

### Shell Commands

```
> Create a sandbox and check the OS version with `cat /etc/os-release`
```

### File Operations

```
> Create a sandbox, write a Python script to /tmp/calc.py, then execute it
```

### Pause & Resume

```
> Create a sandbox, set x=42, pause it, then resume and check if x is still 42
```

## Development & Debugging

### Test with MCP Inspector

```bash
cd mcp-server
mcp dev src/mcp_cubesandbox/server.py
```

This opens the MCP Inspector in your browser, letting you test each tool interactively.

### Run Tests

```bash
cd mcp-server
pip install -e ".[dev]"
pytest
```

## Caveats

- **Sandbox tracking is session-scoped**: The MCP server tracks sandboxes in memory. If the MCP server process restarts, previously created sandboxes become untracked (but still running until their TTL expires). Use `sandbox_list` on the CubeAPI directly to find orphaned sandboxes.
- **Variables persist within a sandbox**: `run_code` calls within the same sandbox share a Python kernel — variables defined in one call are available in the next.
- **Cleanup on exit**: The MCP server automatically kills all tracked sandboxes when it shuts down (via `atexit`). This prevents resource leaks but means sandboxes do not survive MCP server restarts.
- **Sync SDK**: The `cubesandbox` Python SDK is synchronous. FastMCP runs sync tools in a thread pool, so this does not block the MCP server.
- **SSL certificates**: When using CubeSandbox's mkcert self-signed certificates, set `SSL_CERT_FILE` so the SDK can verify sandbox TLS connections.

## References

- [CubeSandbox Quick Start](../quickstart.md)
- [CubeSandbox Architecture](../architecture/overview.md)
- [MCP Server source](https://github.com/TencentCloud/CubeSandbox/tree/master/mcp-server)
- [Model Context Protocol](https://modelcontextprotocol.io)
- [Claude Code MCP Configuration](https://docs.anthropic.com/en/docs/claude-code/mcp)
