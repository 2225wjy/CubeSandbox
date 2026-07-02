# mcp-cubesandbox

MCP (Model Context Protocol) Server for [CubeSandbox](https://github.com/TencentCloud/CubeSandbox) — lets Claude Code and other MCP-compatible AI tools manage hardware-isolated MicroVM sandboxes.

## Features

| Tool | Description |
|------|-------------|
| `sandbox_create` | Create a new sandbox from a template |
| `sandbox_kill` | Destroy a sandbox and free resources |
| `sandbox_list` | List active sandboxes in this session |
| `sandbox_info` | Get detailed sandbox status and metadata |
| `sandbox_pause` | Pause a sandbox (preserves memory snapshot) |
| `sandbox_resume` | Resume a paused sandbox |
| `run_code` | Execute Python code inside a sandbox |
| `run_command` | Execute a shell command inside a sandbox |
| `read_file` | Read a file from the sandbox filesystem |
| `write_file` | Write a file to the sandbox filesystem |

## Prerequisites

- Python 3.10+
- A running CubeSandbox deployment with CubeAPI reachable
- A sandbox template created (`cubemastercli tpl create-from-image ...`)

## Quick Start

### 1. Install

```bash
cd mcp-server
pip install -e .
```

### 2. Configure environment variables

```bash
export CUBE_API_URL="http://<cube-host>:3000"
export CUBE_TEMPLATE_ID="<your-template-id>"

# Optional
export CUBE_PROXY_NODE_IP="<proxy-node-ip>"   # DNS bypass for remote access
export SSL_CERT_FILE="/root/.local/share/mkcert/rootCA.pem"  # mkcert CA
```

### 3. Configure Claude Code

Add to your project's `.mcp.json` or run the CLI command:

**Option A: `.mcp.json`**

```json
{
  "mcpServers": {
    "cubesandbox": {
      "command": "uv",
      "args": ["run", "--directory", "mcp-server", "mcp-cubesandbox"],
      "env": {
        "CUBE_API_URL": "http://<cube-host>:3000",
        "CUBE_TEMPLATE_ID": "<your-template-id>"
      }
    }
  }
}
```

**Option B: CLI**

```bash
claude mcp add \
  --env CUBE_API_URL="http://<cube-host>:3000" \
  --env CUBE_TEMPLATE_ID="<your-template-id>" \
  --transport stdio \
  cubesandbox \
  -- uv run --directory mcp-server mcp-cubesandbox
```

### 4. Use in Claude Code

Once configured, Claude Code can use the sandbox tools:

```
> Create a sandbox and run "print('hello')" in it

Claude Code will:
1. Call sandbox_create → get sandbox_id
2. Call run_code(sandbox_id, "print('hello')") → get output
3. Call sandbox_kill → clean up
```

## Environment Variables

| Variable | Required | Default | Description |
|----------|:--------:|---------|-------------|
| `CUBE_API_URL` | ✅ | `http://127.0.0.1:3000` | CubeAPI address |
| `CUBE_TEMPLATE_ID` | ✅ | — | Default template ID |
| `E2B_API_KEY` | ❌ | — | Auth key (any value for local deploy) |
| `CUBE_PROXY_NODE_IP` | remote | — | CubeProxy node IP (DNS bypass) |
| `SSL_CERT_FILE` | HTTPS | — | CA certificate path for mkcert |

## Development

```bash
# Run with MCP Inspector for debugging
mcp dev src/mcp_cubesandbox/server.py

# Run tests
pip install -e ".[dev]"
pytest
```

## Architecture

```
Claude Code (MCP Client)
    │ stdio (MCP Protocol)
    ▼
mcp-cubesandbox (this server)
    │ Python SDK (cubesandbox)
    ▼
CubeAPI (:3000) ── Control Plane
    │
CubeMaster → Cubelet → MicroVM (envd)
                           │
CubeProxy ─────────────────┘ Data Plane
```

## License

Apache-2.0
