---
title: Claude Code 集成指南
author: cubesandbox-team
date: 2026-07-01
tags:
  - integration
  - claude-code
  - mcp
lang: zh-CN
---

# Claude Code 集成指南

## 集成目标与版本

[Claude Code](https://docs.anthropic.com/en/docs/claude-code) 是 Anthropic 推出的 AI 驱动的 CLI 开发助手。本指南介绍如何通过 **Model Context Protocol (MCP)** 将 Claude Code 与 CubeSandbox 集成，使 Claude Code 能够在硬件隔离的 MicroVM 沙箱中创建沙箱、执行代码、运行 Shell 命令和管理文件。

| 组件 | 版本 |
|------|------|
| Claude Code | 最新版 (CLI) |
| MCP Server | `mcp-cubesandbox` 0.1.0 |
| CubeSandbox | ≥ 0.3.0 |
| Python | ≥ 3.10 |

## 工作原理

```
Claude Code (MCP 客户端)
    │  stdio (MCP 协议)
    ▼
mcp-cubesandbox (MCP Server)
    │  cubesandbox Python SDK
    ▼
CubeAPI (:3000) ── 控制平面
    │
CubeMaster → Cubelet → KVM MicroVM (envd)
                           │
CubeProxy ─────────────────┘ 数据平面
```

MCP Server 充当"适配器"角色：将 Claude Code 的工具调用翻译为 CubeSandbox 的 REST API 请求。任何支持 MCP 协议的客户端（不仅限于 Claude Code）都可以使用同一个 Server。

## 前置条件

- 已部署的 CubeSandbox 环境，CubeAPI 可达
- 已创建沙箱模板（参见[快速开始](../quickstart.md)）
- 运行 Claude Code 的机器需要 Python 3.10+
- 已安装 [Claude Code CLI](https://docs.anthropic.com/en/docs/claude-code)

## 集成步骤

### 1. 安装 MCP Server

克隆 CubeSandbox 仓库并安装 MCP Server 包：

```bash
git clone https://github.com/TencentCloud/CubeSandbox.git
cd CubeSandbox/mcp-server
pip install -e .
```

### 2. 配置环境变量

设置 CubeSandbox 标准环境变量：

```bash
export CUBE_API_URL="http://<cube-host>:3000"
export CUBE_TEMPLATE_ID="<你的模板 ID>"

# 可选 — 远程访问或 mkcert HTTPS 场景
export CUBE_PROXY_NODE_IP="<代理节点 IP>"
export SSL_CERT_FILE="/root/.local/share/mkcert/rootCA.pem"
```

| 变量 | 必需 | 说明 |
|------|:---:|------|
| `CUBE_API_URL` | ✅ | CubeAPI 地址（端口 3000） |
| `CUBE_TEMPLATE_ID` | ✅ | 创建沙箱时使用的默认模板 ID |
| `E2B_API_KEY` | ❌ | 认证密钥（本地部署可填任意非空值） |
| `CUBE_PROXY_NODE_IP` | 远程 | CubeProxy 节点 IP，用于 DNS 旁路 |
| `SSL_CERT_FILE` | HTTPS | mkcert CA 证书路径 |

### 3. 配置 Claude Code

**方式 A：项目级 `.mcp.json`**（推荐）

在项目根目录创建 `.mcp.json`：

```json
{
  "mcpServers": {
    "cubesandbox": {
      "command": "python",
      "args": ["-m", "mcp_cubesandbox.server"],
      "env": {
        "CUBE_API_URL": "http://<cube-host>:3000",
        "CUBE_TEMPLATE_ID": "<你的模板 ID>"
      }
    }
  }
}
```

如果使用 `uv`：

```json
{
  "mcpServers": {
    "cubesandbox": {
      "command": "uv",
      "args": ["run", "--directory", "/path/to/CubeSandbox/mcp-server", "mcp-cubesandbox"],
      "env": {
        "CUBE_API_URL": "http://<cube-host>:3000",
        "CUBE_TEMPLATE_ID": "<你的模板 ID>"
      }
    }
  }
}
```

**方式 B：CLI 命令**

```bash
claude mcp add \
  --env CUBE_API_URL="http://<cube-host>:3000" \
  --env CUBE_TEMPLATE_ID="<你的模板 ID>" \
  --transport stdio \
  cubesandbox \
  -- python -m mcp_cubesandbox.server
```

### 4. 验证集成

启动 Claude Code 并确认 MCP Server 已连接：

```bash
claude
```

在 Claude Code 中输入：

```
> 列出可用的沙箱
```

Claude Code 应调用 `sandbox_list` 工具并返回结果。

## 可用工具

| 工具 | 说明 |
|------|------|
| `sandbox_create` | 创建新沙箱，返回 `sandbox_id` |
| `sandbox_kill` | 销毁沙箱，释放资源 |
| `sandbox_list` | 列出当前会话中所有活跃沙箱 |
| `sandbox_info` | 获取沙箱详细状态（状态、CPU、内存） |
| `sandbox_pause` | 暂停沙箱（保留内存快照） |
| `sandbox_resume` | 恢复已暂停的沙箱 |
| `run_code` | 在沙箱中执行 Python 代码，变量跨调用持久化 |
| `run_command` | 在沙箱中执行 Shell 命令 |
| `read_file` | 读取沙箱内文件 |
| `write_file` | 向沙箱内文件写入内容 |

## 使用示例

### 基础用法：创建、执行、销毁

```
> 创建一个沙箱，在里面运行 print("Hello from CubeSandbox!")，完成后销毁
```

Claude Code 会自动：
1. 调用 `sandbox_create` → 获得 `sandbox_id`
2. 调用 `run_code` 传入 ID 和 Python 代码
3. 展示执行结果
4. 调用 `sandbox_kill` 清理资源

### Shell 命令

```
> 创建沙箱，用 cat /etc/os-release 查看操作系统版本
```

### 文件操作

```
> 创建沙箱，写一个 Python 脚本到 /tmp/calc.py，然后执行它
```

### 暂停与恢复

```
> 创建沙箱，设 x=42，暂停沙箱，然后恢复并检查 x 是否仍然是 42
```

## 开发与调试

### 使用 MCP Inspector 测试

```bash
cd mcp-server
mcp dev src/mcp_cubesandbox/server.py
```

这会在浏览器中打开 MCP Inspector，可以交互式地测试每个工具。

### 运行测试

```bash
cd mcp-server
pip install -e ".[dev]"
pytest
```

## 注意事项

- **沙箱跟踪是会话级的**：MCP Server 在内存中跟踪沙箱。如果 MCP Server 进程重启，之前创建的沙箱将变为未跟踪状态（但仍会运行直到 TTL 过期）。可直接通过 CubeAPI 的 `sandbox_list` 查找孤立的沙箱。
- **变量在同一沙箱内持久化**：同一沙箱中的 `run_code` 调用共享 Python 内核 — 一次调用中定义的变量在下次调用中仍然可用。
- **退出时自动清理**：MCP Server 关闭时会自动销毁所有跟踪的沙箱（通过 `atexit`），防止资源泄漏，但也意味着沙箱无法在 MCP Server 重启后存活。
- **同步 SDK**：`cubesandbox` Python SDK 是同步的。FastMCP 在线程池中运行同步工具，不会阻塞 MCP Server。
- **SSL 证书**：使用 CubeSandbox 的 mkcert 自签名证书时，需设置 `SSL_CERT_FILE` 以便 SDK 验证沙箱的 TLS 连接。

## 参考资料

- [CubeSandbox 快速开始](../quickstart.md)
- [CubeSandbox 架构设计](../architecture/overview.md)
- [MCP Server 源码](https://github.com/TencentCloud/CubeSandbox/tree/master/mcp-server)
- [Model Context Protocol](https://modelcontextprotocol.io)
- [Claude Code MCP 配置](https://docs.anthropic.com/en/docs/claude-code/mcp)
