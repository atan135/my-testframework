# qamcp

`qamcp` 是 QA Test Framework 的本地 MCP server 和命令行工具。AI Agent 可以通过它连接 QA register server，发现在线 Unity 实例、搜索 `[QaTest]` 方法、执行单个测试指令、执行请求序列，并读取或订阅执行结果。

## 运行条件

- Rust toolchain，用于本地开发和构建 exe。
- QA register server 已启动，默认地址为 `http://localhost:3000`。
- Unity 工程已进入 Play Mode，并通过 Unity QA 客户端连接到 register server。

如果 register server 不在默认地址，通过 `config` 子命令保存：

```powershell
qamcp config set serverUrl http://localhost:3000
qamcp config
```

配置会写入 `qamcp.exe` 同目录的 `qamcp.config.json`。使用 `dist/qamcp.exe` 时，配置文件位于 `dist/qamcp.config.json`。

`qamcp` 会为当前 MCP 进程生成一个稳定的控制端 ID，用于 server 侧控制锁。需要跨进程复用同一个控制端身份时，可以显式设置：

```powershell
$env:QA_CONTROLLER_ID='my-agent-session'
```

## 本地开发

```powershell
cargo fmt --check
cargo check
cargo test
cargo run -- mcp
```

也可以用 npm 脚本作为本地快捷入口；这些脚本只调用 Cargo，不再安装或打包 Node 运行时：

```powershell
npm run check
npm test
npm start
```

## 源码结构

```text
src/
  main.rs           进程入口；mcp 子命令进入 MCP stdio，无参数或 CLI 子命令进入命令行模式
  config.rs         qamcp.config.json 读写、serverUrl 规范化
  constants.rs      版本号、默认超时和 controller id
  mcp.rs            MCP stdio JSON-RPC、qa_* tool 注册和事件会话状态
  cli/              CLI 参数解析和输出
  qa/               QA register server HTTP API 与 WebSocket 控制协议
scripts/
  build-release.ps1 Cargo release 构建并复制 dist/qamcp.exe
```

`qamcp` 使用 stdio MCP transport。作为 MCP server 使用时必须显式传入 `mcp` 子命令；无参数运行会打印 CLI 帮助。如果带 CLI 子命令运行，则会直接连接 QA register server 并把结果输出到终端。

## 命令行调用

默认输出为结构化 JSON；需要人类可读摘要或表格时加 `--text` 或 `--human`。

```powershell
qamcp        # 显示帮助
qamcp mcp    # 启动 stdio MCP server
qamcp health
qamcp clients
qamcp config set serverUrl http://localhost:3000
qamcp search --query Ping --limit 10
qamcp describe --query "Ping|GetState"
qamcp results --status failed --limit 20
qamcp run --client unity-editor-1 --tool "QaTestSample.Ping()"
qamcp run -c unity-editor-1 --tool "QaTestSample.SelectStartupServerAndLanguage(System.String,System.String)" -a 42513 -a cn
qamcp run -c unity-editor-1 --tool "QaRecord.PlayUiOperationReplay(System.String,System.String,System.Int32)" -a "" --arg-file ".\record.txt" -a 1
qamcp sequence -c unity-editor-1 --steps "[{\"methodId\":\"QaTestSample.Ping()\"}]"
qamcp watch --event qa_result,sequence_finished --duration 5000
qamcp clients --text
```

`run` 方法参数优先使用重复 `-a` / `--arg` 按顺序传入，例如 `-a val1 -a val2`。大文本参数可使用 `--arg-file <path>`，qamcp 会在本机读取文件内容，并把内容作为该位置的一个字符串参数发送给 Unity 客户端；`-a` 和 `--arg-file` 可以按出现顺序混用。旧的 `--args <json-array>` 仍兼容已有脚本，但在 PowerShell 和 cmd 中容易被引号/转义规则改写，不再作为推荐用法。

可用子命令：`health`、`clients`、`config`、`search`、`describe`、`results`、`run`、`sequence`、`stop`、`wait`、`watch`、`mcp`。

方法发现相关命令只保留 `search` 和 `describe`；旧的 `tools`、`tool`、`methods`、`find`、`method` 不再作为 CLI command 使用。

## 构建 exe

Windows 上推荐用 Rust release 构建本地 exe，后续 MCP 客户端可以直接把 `command` 指向生成文件：

```powershell
cargo build --release
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item .\target\release\qamcp.exe .\dist\qamcp.exe -Force
```

或使用仓库脚本：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-release.ps1
```

构建产物：

```text
dist/qamcp.exe
```

这个 exe 内置 `qamcp` MCP server，不包含 Node.js runtime，不需要目标机器安装本仓库源码、Node.js 或全局 npm 命令。它不会内置 QA register server，也不会启动 Unity 客户端。

## MCP 客户端配置

配置 MCP 客户端前，先保存 QA register server 地址：

```powershell
.\dist\qamcp.exe config set serverUrl http://localhost:3000
```

推荐使用 exe 的配置。`command` 建议写绝对路径：

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

## Tools

- `qa_health`: 检查 QA register server 健康状态。
- `qa_list_unity_clients`: 列出当前在线 Unity 客户端，可选择是否返回完整方法列表。
- `qa_find_methods`: 按客户端、方法名、方法 ID、声明类型、描述或返回类型搜索 `[QaTest]` 方法。
- `qa_get_method`: 按精确 methodId、短 methodId 或方法名获取一个或多个 `[QaTest]` 方法详情，多个查询用 `|` 分隔。
- `qa_get_results`: 查询最近执行记录，支持按 `clientId`、`status`、`sequenceId` 过滤。
- `qa_execute_method`: 通过 WebSocket 执行单个 `[QaTest]` 方法并等待 `qa_result`。
- `qa_execute_sequence`: 顺序执行多个 `[QaTest]` 方法并等待 `sequence_finished`，可用 `stepDelayMs` 设置相邻步骤间隔。
- `qa_stop_execution`: 停止正在运行的单次执行。
- `qa_stop_sequence`: 停止正在运行的请求序列。
- `qa_watch_events`: 临时监听一段时间内的 QA WebSocket 事件并一次性返回。
- `qa_wait_for_result`: 按 `requestId`、`sequenceId`、`clientId` 或 `methodId` 等条件等待结果。
- `qa_open_event_session`: 打开一条可跨多次 MCP 调用轮询的 QA WebSocket 事件会话。
- `qa_poll_event_session`: 轮询事件会话，读取新事件和 `nextCursor`。
- `qa_close_event_session`: 关闭事件会话并释放 WebSocket 连接。
