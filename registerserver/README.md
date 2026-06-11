# QA Register Server

`registerserver` 是 QA Test Framework 的注册与调度服务。它负责接收 Unity 客户端注册的 `[QaTest]` 方法，向 Web 控制台和 MCP 工具暴露在线客户端列表，并把执行请求转发到指定 Unity 实例。

## 目录结构

- `rustserver/`: Rust + Axum + Tokio 服务端，提供 HTTP API、WebSocket 协议、执行调度、请求序列和静态 Web 控制台托管能力；作为唯一主实现维护。
- `server/`: Node.js + Express + WebSocket 旧服务端，已冻结为 legacy，仅保留历史参考和紧急回退入口，不再同步新能力。
- `client/`: Vue 3 + Vite + Element Plus Web 控制台，用于查看 Unity 客户端、执行方法、编排请求序列和查看历史结果。

## 运行条件

- Rust toolchain，用于运行主服务端。
- Node.js 18+，用于构建 Web 控制台；只有临时回退到 legacy Node 服务端时才需要安装 `server/` 依赖。
- Unity 客户端工程已集成 `QaTestClient`，并连接到本服务的 WebSocket 地址。

默认端口是 `3000`。如需修改端口：

```powershell
$env:PORT=3001
```

WebSocket 故障处理相关配置：

```powershell
$env:EXECUTION_TIMEOUT_MS=20000
$env:UNITY_HEARTBEAT_STALE_MS=45000
$env:WS_HEARTBEAT_INTERVAL_MS=15000
```

- `EXECUTION_TIMEOUT_MS`: 单次执行和请求序列单步超时，默认 20 秒。
- `UNITY_HEARTBEAT_STALE_MS`: Unity 业务 heartbeat 超时后标记不可用，默认 45 秒。
- `WS_HEARTBEAT_INTERVAL_MS`: WebSocket ping/pong 检查间隔，默认 15 秒。

## 安装依赖

安装前端依赖：

```powershell
cd client
npm install
```

如果需要临时回退到 legacy Node 服务端，再安装 Node 服务端依赖：

```powershell
cd server
npm install
```

Rust 服务端使用 Cargo 管理依赖，首次运行或构建时会自动下载依赖：

```powershell
cd rustserver
cargo check
```

## 开发运行

启动 Rust 主服务端：

```powershell
cd rustserver
cargo run
```

如需临时回退到 legacy Node 服务端：

```powershell
cd server
npm run dev
```

启动前端开发服务器：

```powershell
cd client
npm run dev
```

前端开发服务器默认监听 `http://localhost:5173`，并通过 Vite proxy 转发 `/api` 和 `/ws` 到 `http://localhost:3000`。服务端默认监听 `http://localhost:3000`，WebSocket 路径为 `/ws`。

Rust 服务端和 legacy Node 服务端默认都监听 `3000` 端口；同一时间只需要启动其中一个。后续协议和状态机只以 Rust 服务端为准，Node legacy 不再同步新能力或常规修复。

## 生产运行

可以用脚本一键构建 Web 控制台、Rust release exe，并复制到 `release/qa-register/`：

```powershell
.\build-release.ps1
```

如果本机前端依赖已安装，也可以跳过 `npm ci`：

```powershell
.\build-release.ps1 -SkipNpmCi
```

脚本会复制：

```text
release/qa-register/
  qa-register-rustserver.exe
  install-scheduled-task.ps1
  client/
    dist/
```

脚本还会生成：

```text
release/qa-register.zip
```

脚本不会复制 `.env`，也不会把 `.env` 或运行日志打进 zip。线上环境请手动创建或编辑 `.env`，避免 token 等敏感配置误提交。

线上 Windows 解压后，可以用管理员 PowerShell 注册开机自启计划任务：

```powershell
cd C:\deploy\qa-register
powershell -NoProfile -ExecutionPolicy Bypass -File .\install-scheduled-task.ps1
```

脚本默认注册 `QA Register Server` 任务，工作目录为脚本所在目录，并立即启动服务。

如需自定义输出路径：

```powershell
.\build-release.ps1 -OutputDir .\release\qa-register -ZipPath .\release\qa-register.zip
```

也可以手动构建。先构建前端：

```powershell
cd client
npm run build
```

再启动 Rust 主服务端：

```powershell
cd ..\rustserver
cargo run --release
```

如需临时回退到 legacy Node 服务端：

```powershell
cd ..\server
npm start
```

服务端会托管 `client/dist`，浏览器访问 `http://localhost:3000` 即可打开控制台。Rust 服务端和 legacy Node 服务端都使用同一份 `client/dist`，但最新 Web 控制台行为只保证以 Rust 服务端为准。

## HTTP API

- `GET /api/health`: 返回服务状态、运行时间、Unity 客户端数量和 Web 客户端数量。
- `GET /api/unity-clients`: 返回当前在线 Unity 客户端和它们注册的 `[QaTest]` 方法。
- `GET /api/results`: 返回最近执行记录，最多保留 200 条。
- `POST /api/unity-clients/:clientId/execute`: 向指定 Unity 客户端发送单个执行请求。

单次执行示例：

```powershell
curl.exe -X POST http://localhost:3000/api/unity-clients/<clientId>/execute `
  -H "Content-Type: application/json" `
  -d "{\"methodId\":\"<methodId>\",\"arguments\":[\"arg1\"]}"
```

## WebSocket 协议

WebSocket 入口是 `/ws`，通过 `role` 区分连接类型：

- `ws://localhost:3000/ws?role=unity`: Unity 客户端注册和接收执行命令。
- `ws://localhost:3000/ws?role=web`: Web 控制台或 MCP 工具监听事件、发送执行请求。

Unity 客户端连接后发送 `register` 消息，服务端记录 `clientId`、客户端名称、IP 地址、平台、Unity 版本、方法列表、业务可用状态、Unity 本地 busy 状态和控制锁状态。`heartbeat` 会持续同步 `busy`、`currentRequestId` 和 `currentMethodName`，服务端对 Web/qamcp 公开为 `clientBusy`、`currentRequestId` 和 `currentMethodName`，并在本地 busy 状态变化时广播 `unity_state_changed`。Web 客户端连接后会收到 `snapshot`，后续会收到 `unity_registered`、`unity_state_changed`、`unity_disconnected`、`unity_unavailable`、`unity_available`、`client_locked`、`client_unlocked`、`execution_started`、`execute_accepted`、`execute_rejected`、`qa_result`、`qa_result_late`、`sequence_started`、`sequence_step_started`、`sequence_step_result`、`sequence_finished`、`stop_accepted`、`stop_rejected` 等事件。

网络断线、业务 heartbeat 超时、执行超时和控制锁策略见 `../docs/WebSocket故障处理策略.md`。

## 请求序列

WebSocket `execute_sequence` 消息可以顺序执行多个方法：

- `clientId`: 目标 Unity 客户端。
- `steps`: 请求步骤数组，每步包含 `methodId`、可选 `methodName` 和 `arguments`。
- `stopOnFailure`: 默认为 `true`，失败后停止后续步骤。
- `stepDelayMs`: 相邻步骤间隔，范围 `0` 到 `300000` 毫秒。

## 校验

当前仓库没有统一测试脚本。提交前至少运行：

```powershell
cd rustserver
cargo fmt
cargo check
cargo test
```

```powershell
cd client
npm run build
```

Node legacy 已冻结，不参与常规开发和发布验证。只有确实需要临时回退运行时才检查：

```powershell
cd ..\server
node --check src/index.js
```

Node legacy 的定位、风险和回退步骤见：`../docs/NodeLegacy服务端说明.md` 和 `server/README.md`。

## 相关项目

- Unity 客户端：注册 `[QaTest]` 方法并执行服务端命令。
- `qamcp`: 面向 AI Agent 的 MCP 工具层，复用本服务的 HTTP 和 WebSocket 能力。
