# QAMCP：Agent 工具使用指南

`qamcp` 是 QA Test Framework 的 stdio MCP 工具服务。AI Agent 可以通过它调用现有 QA register server 的 HTTP/WebSocket 能力，完成 Unity 实例发现、方法搜索、单次执行、请求序列执行和结果查询。

## 前置条件

先启动 QA register server，并让 Unity 客户端进入 Play Mode 完成连接。Editor 默认启用 QA 功能；如果是构建后的 Player，需要显式传入 `--qa-test-enable` 或使用 `QA_TEST_ENABLED=1` 等方式开启；`--qa-enable` 仍作为旧别名兼容：

```powershell
cd registerserver/rustserver
cargo run
```

默认 QA server 地址是 `http://localhost:3000`。如果服务端不在默认地址，通过 qamcp 配置文件保存：

```powershell
qamcp config set serverUrl http://localhost:3000
qamcp config
```

配置会写入 qamcp 同目录的 `qamcp.config.json`。使用 `qamcp/dist/qamcp.exe` 时，配置文件位于 `qamcp/dist/qamcp.config.json`。

`qamcp` 会为当前 MCP 进程生成一个控制端 ID，用于 register server 的一对一控制锁。需要跨进程复用同一个身份时，可以显式设置：

```powershell
$env:QA_CONTROLLER_ID='my-agent-session'
```

## 本地校验

```powershell
cd qamcp
cargo fmt --check
cargo check
cargo test
```

`qamcp` 使用 stdio MCP transport。作为 MCP server 使用时必须显式传入 `mcp` 子命令；无参数运行会打印 CLI 帮助。

## 对外交付

对外使用时，推荐交付 Windows exe。`qamcp` 当前使用 Rust release 构建单文件可执行程序：

```powershell
cd qamcp
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-release.ps1
```

构建产物：

```text
qamcp/dist/qamcp.exe
```

MCP 客户端只需要配置这个 exe，不需要目标机器安装本仓库源码、Node.js 或全局 npm 命令。exe 不包含 QA register server，也不会启动 Unity 客户端；运行时从 exe 同目录的 `qamcp.config.json` 读取 serverUrl，未配置时默认连接 `http://localhost:3000`。

## MCP 客户端配置

配置 MCP 客户端前，先保存 QA register server 地址：

```powershell
cd qamcp/dist
.\qamcp.exe config set serverUrl http://localhost:3000
```

推荐 exe 配置，`command` 使用绝对路径：

```json
{
  "mcpServers": {
    "qamcp": {
      "command": "C:\\project\\qatestframework\\qamcp\\dist\\qamcp.exe",
      "args": ["mcp"]
    }
  }
}
```

## 可用 Tools

- `qa_health`: 检查 QA server 是否可用
- `qa_list_unity_clients`: 查看在线 Unity 实例
- `qa_find_methods`: 搜索 Unity 上已注册的 `[QaTest]` 方法
- `qa_get_method`: 按精确 methodId、短 methodId 或方法名获取一个或多个 `[QaTest]` 方法详情，多个查询用 `|` 分隔
- `qa_get_results`: 查询最近执行记录
- `qa_execute_method`: 执行单个 `[QaTest]` 方法并等待返回
- `qa_execute_sequence`: 顺序执行一组请求并等待汇总结果，可用 `stepDelayMs` 设置相邻步骤间隔
- `qa_stop_execution`: 停止正在运行的单次执行
- `qa_stop_sequence`: 停止正在运行的请求序列
- `qa_watch_events`: 临时监听一段时间内的 QA WebSocket 事件并一次性返回
- `qa_wait_for_result`: 按 `requestId`、`sequenceId`、`clientId` 或 `methodId` 等条件等待结果
- `qa_open_event_session`: 打开一条可跨多次 MCP 调用轮询的 QA WebSocket 事件会话
- `qa_poll_event_session`: 轮询事件会话，读取新事件和 `nextCursor`
- `qa_close_event_session`: 关闭事件会话并释放 WebSocket 连接

## 长连接事件用法

如果只想临时观察一小段时间，可以让 AI 调用 `qa_watch_events`：

```text
用 qamcp 监听 10 秒 QA WebSocket 事件，只看 qa_result 和 sequence_finished。
```

如果需要跨多轮对话持续观察事件，使用会话式工具：

1. 调用 `qa_open_event_session`，得到 `sessionId`。
2. 定期调用 `qa_poll_event_session`，第一次 `cursor` 可不传，之后传上次返回的 `nextCursor`。
3. 不需要继续监听时调用 `qa_close_event_session`。

示例对话：

```text
用 qamcp 打开 QA 事件会话，只缓存 qa_result、sequence_finished、unity_registered。
```

```text
用 qamcp 轮询刚才的事件会话，最多返回 50 条事件。
```

```text
用 qamcp 关闭刚才的事件会话。
```

事件会话默认 10 分钟过期，默认最多缓存 500 条事件。如果事件产生速度超过缓存容量，轮询结果会返回 `lostEventCount` 和 `droppedEvents`，表示有旧事件已被丢弃。

## curl 调试

这些命令直接调用 QA register server，不经过 MCP。它们适合排查 server 和 Unity 客户端连接状态，也可以作为 API 文档给非 MCP 使用者参考。

健康检查：

```powershell
curl.exe http://localhost:3000/api/health
```

查看在线 Unity 客户端：

```powershell
curl.exe http://localhost:3000/api/unity-clients
```

查看最近执行记录：

```powershell
curl.exe http://localhost:3000/api/results
```

执行单个 QaTest 方法：

```powershell
curl.exe -X POST http://localhost:3000/api/unity-clients/<clientId>/execute `
  -H "Content-Type: application/json" `
  -d "{\"methodId\":\"<methodId>\",\"methodName\":\"<methodName>\",\"arguments\":[\"arg1\"]}"
```

请求序列执行使用 WebSocket 协议，建议通过 MCP 的 `qa_execute_sequence` 或 Web 控制台执行。需要控制相邻指令触发间隔时，给 `qa_execute_sequence` 传入 `stepDelayMs`，单位毫秒，范围是 `0` 到 `300000`。
