# RegisterServer：服务端 API 与 WebSocket 协议

`registerserver` 是 QA Test Framework 的注册与调度服务，包含服务端和网页控制台两部分：

- `registerserver/rustserver`: Rust + Axum + Tokio 主服务端。
- `registerserver/server`: Node.js + Express + WebSocket 旧服务端，已冻结为 legacy，仅作为历史行为参考和紧急回退入口。
- `registerserver/client`: Vue 3 + Vite + Element Plus 网页控制台。

它负责接收 Unity 客户端注册的 `[QaTest]` 方法，把在线实例和方法列表展示给 Web 控制台或 MCP 工具，并把执行请求转发到指定 Unity 实例。

## 运行环境

- Rust toolchain。
- Node.js 18+ 和 npm，用于构建网页控制台；只有临时回退到 legacy Node 服务端时才需要安装 `registerserver/server` 依赖。
- 至少一个已集成 `unityclient` 的 Unity 工程；Editor 进入 Play Mode 后默认连接到本服务，Player 需要显式启用 QA 功能。

完整配置项见：[配置参考-端口超时鉴权与执行限制](./配置参考-端口超时鉴权与执行限制.md)。
本地开发可复制 `registerserver/rustserver/.env.example` 为 `.env`，Rust server 启动时会自动加载。线上 Windows 部署时也可以把 `.env` 放在 exe 同目录；当前工作目录没有 `.env` 时，server 会自动尝试读取 exe 同目录的 `.env`。

默认服务端口是 `3000`。如需改端口，在启动服务端前设置：

```powershell
$env:PORT=3001
```

默认监听所有网卡但限制内网来源，便于同一内网内的 Unity、Web 控制台和 qamcp 访问：

```powershell
$env:QA_LISTEN_HOST="0.0.0.0"
$env:QA_ACCESS_SCOPE="private"
```

- `QA_LISTEN_HOST`: HTTP 和 WebSocket 监听地址，默认 `0.0.0.0`。
- `QA_ACCESS_SCOPE`: 访问范围，默认 `private`，只允许本机、RFC1918 私网、链路本地和 IPv6 内网来源。需要取消来源限制时显式设置为 `unrestricted`。

WebSocket 故障处理相关配置：

```powershell
$env:EXECUTION_TIMEOUT_MS=20000
$env:UNITY_HEARTBEAT_STALE_MS=45000
$env:WS_HEARTBEAT_INTERVAL_MS=15000
```

- `EXECUTION_TIMEOUT_MS`: 单次执行和请求序列单步超时，默认 20 秒。
- `UNITY_HEARTBEAT_STALE_MS`: Unity 业务 heartbeat 超时后标记不可用，默认 45 秒。
- `WS_HEARTBEAT_INTERVAL_MS`: WebSocket ping/pong 检查间隔，默认 15 秒。

Web 控制台登录 token：

```powershell
$env:QA_WEB_CONSOLE_TOKEN="replace-with-internal-token"
```

- `QA_WEB_CONSOLE_TOKEN`: 可选。未配置时 Web 控制台保持免登录；配置后浏览器打开控制台需要输入 token。
- 该 token 只用于 Web 控制台轻量登录。Unity 客户端和 qamcp 在内网部署下不需要额外传 token。

Web 控制台构建产物目录：

```powershell
$env:QA_CLIENT_DIST_DIR="C:\deploy\qa-register\client\dist"
```

未设置时，服务端优先使用 exe 同目录下的 `client/dist`；本地开发回退到仓库中的 `registerserver/client/dist`。

服务端会向 stdout 输出 JSON line 结构化日志。执行相关事件会携带 `requestId`、`clientId`、`controllerId` 和 `sequenceId` 等字段，完整说明见：[配置参考-端口超时鉴权与执行限制](./配置参考-端口超时鉴权与执行限制.md)。

需要落地到文件时，可设置 `QA_LOG_DIR` 和 `QA_LOG_PREFIX`。服务端会继续输出 stdout，并额外按天切分写入 JSON line 日志文件。

## 目录结构

```text
registerserver/
  client/             Vue 网页控制台
    src/views/        控制台、请求序列、执行记录页面
    src/composables/  WebSocket 状态和数据 store
    vite.config.js    开发代理配置
  rustserver/         Rust + Axum + Tokio 主服务端
    src/main.rs       服务启动、路由注册、生命周期日志
    src/api.rs        HTTP API
    src/websocket.rs  WebSocket 协议入口
    src/execution.rs  单次执行调度和结果收敛
    src/sequence.rs   请求序列调度
    src/archive/      可选 MariaDB/MySQL 执行历史旁路存档
  server/             Express + ws 旧服务端，已冻结，仅保留历史参考和紧急回退
    README.md         Node legacy 定位和回退说明
    src/index.js      legacy HTTP API、WebSocket 协议、执行调度
```

## 开发启动

安装前端依赖：

```powershell
cd registerserver/client
npm install
```

启动 Rust 主服务端：

```powershell
cd registerserver/rustserver
cargo run
```

启动前端开发服务器：

```powershell
cd registerserver/client
npm run dev
```

开发地址默认是 `http://localhost:5173`。Vite 会把 `/api` 和 `/ws` 代理到 `http://localhost:3000`。

## 生产启动

生产端 Windows 不需要安装 Rust 或 Node。推荐先在开发机或 CI 构建产物，再把 exe、`.env` 和 Web 控制台 `dist` 拷贝到生产端。

构建机可以一键生成发布目录：

```powershell
cd registerserver
.\build-release.ps1
```

默认输出：

```text
registerserver/
  release/
    qa-register.zip
    qa-register/
      qa-register-rustserver.exe
      install-scheduled-task.ps1
      client/
        dist/
          index.html
          assets/
```

脚本会执行 Web 控制台 build、Rust release build，复制 exe 与 `client/dist`，并把 `release/qa-register` 打包成 `release/qa-register.zip`。脚本不会复制 `.env`，也不会把 `.env` 或运行日志打进 zip；生产端请手动创建或编辑 `.env`，避免 token 等敏感配置误提交到 Git。

生产端解压后，可以用管理员 PowerShell 注册开机自启计划任务：

```powershell
cd C:\deploy\qa-register
powershell -NoProfile -ExecutionPolicy Bypass -File .\install-scheduled-task.ps1
```

脚本默认注册 `QA Register Server` 任务，使用脚本所在目录作为工作目录，并立即启动服务。后续可以在任务计划程序的根目录查看该任务。

如果本机前端依赖已安装，可以跳过 `npm ci`：

```powershell
.\build-release.ps1 -SkipNpmCi
```

如需自定义输出路径：

```powershell
.\build-release.ps1 -OutputDir .\release\qa-register -ZipPath .\release\qa-register.zip
```

也可以手动构建。构建机先构建网页控制台：

```powershell
cd registerserver/client
npm run build
```

再构建 Rust server exe：

```powershell
cd ../rustserver
cargo build --release
```

把以下产物拷贝到生产端同一个目录：

```text
qa-register/
  qa-register-rustserver.exe
  install-scheduled-task.ps1
  .env
  client/
    dist/
      index.html
      assets/
```

来源文件对应关系：

- `qa-register-rustserver.exe`: 来自 `registerserver/rustserver/target/release/qa-register-rustserver.exe`。
- `client/dist`: 来自 `registerserver/client/dist`。
- `.env`: 可从 `registerserver/rustserver/.env.example` 复制后按生产环境修改。

生产端 `.env` 示例：

```env
PORT=3000
QA_LISTEN_HOST=0.0.0.0
QA_ACCESS_SCOPE=private
QA_WEB_CONSOLE_TOKEN=replace-with-internal-token
QA_CLIENT_DIST_DIR=
QA_LOG_DIR=logs
QA_LOG_PREFIX=registerserver
```

这种目录结构下，`QA_CLIENT_DIST_DIR` 可以留空，服务端会自动使用 exe 同目录下的 `client/dist`。如果 Web 控制台文件放在其他目录，再把 `QA_CLIENT_DIST_DIR` 改成绝对路径。

生产端启动：

```powershell
cd C:\deploy\qa-register
.\qa-register-rustserver.exe
```

启动后访问：

```text
http://<生产端内网IP>:3000
```

服务端会托管 Web 控制台构建产物。直接访问 `/history`、`/sequences` 等前端路由时，服务端会回退到 `index.html`。

## Web 控制台

控制台包含三个页面：

- `http://localhost:3000/`: 测试控制台，查看在线 Unity 实例、方法列表和执行单个方法。
- `http://localhost:3000/sequences`: 请求序列，编排多个方法顺序执行，支持失败即停和步骤间隔。
- `http://localhost:3000/history`: 执行记录，查看最近执行状态、输出、错误、耗时和时间。

网页端会建立 `ws://<host>/ws?role=web` 长连接。连接断开后会每 2 秒重连，并通过 HTTP API 刷新当前快照。

如果服务端配置了 `QA_WEB_CONSOLE_TOKEN`，浏览器控制台会先显示登录页。登录成功后服务端写入 HttpOnly cookie，控制台 WebSocket 握手会校验该 cookie。qamcp 和 Unity 连接不受这个登录 token 影响。

## HTTP API

### 健康检查

```http
GET /api/health
```

返回：

```json
{
  "ok": true,
  "uptime": 123.45,
  "unityClientCount": 1,
  "webClientCount": 1,
  "controllerCount": 1,
  "executionTimeoutMs": 20000,
  "unityHeartbeatStaleMs": 45000,
  "webConsoleAuthRequired": true,
  "listenHost": "0.0.0.0",
  "accessScope": "private",
  "clientDistDir": "C:\\deploy\\qa-register\\client\\dist"
}
```

### Web 控制台登录状态

```http
GET /api/web-auth
```

返回当前服务是否要求浏览器控制台 token，以及本次请求是否已通过 cookie 认证：

```json
{
  "tokenRequired": true,
  "authenticated": false
}
```

配置了 `QA_WEB_CONSOLE_TOKEN` 时，浏览器通过 `POST /api/web-login` 提交 `{ "token": "..." }`，成功后服务端写入 HttpOnly cookie；`POST /api/web-logout` 会清除该 cookie。Unity 和 qamcp 不使用这组登录接口。

### 在线 Unity 实例

```http
GET /api/unity-clients
```

返回当前在线客户端和它们注册的方法列表：

```json
{
  "clients": [
    {
      "clientId": "client-id",
      "name": "Game@Device",
      "ipAddress": "192.168.1.50",
      "ipAddresses": ["192.168.1.50"],
      "remoteAddress": "192.168.1.50",
      "platform": "WindowsEditor",
      "unityVersion": "2022.3.62f1",
      "deviceName": "DESKTOP-001",
      "operatingSystem": "Windows 11  (10.0.22631) 64bit",
      "methods": [],
      "connectedAt": "2026-04-28T08:00:00.000Z",
      "lastSeenAt": "2026-04-28T08:00:10.000Z",
      "availabilityChangedAt": "2026-04-28T08:00:00.000Z",
      "online": true,
      "available": true,
      "unavailableReason": "",
      "running": false,
      "clientBusy": false,
      "currentRequestId": "",
      "currentMethodName": "",
      "lock": null
    }
  ]
}
```

### 执行历史

```http
GET /api/results
```

返回最近 200 条执行记录。相同 `requestId` 的 running 记录会在结果返回后被更新。

这部分历史是内存中的最近视图，服务重启后重新累计。长期追溯可以使用 `QA_LOG_DIR` 配置的 JSON line 日志文件；需要长期查询和审计时，可开启可选 MariaDB/MySQL 执行历史存档。数据库存档只做旁路写入，不改变 `/api/results` 的返回来源。

### 截图/附件 Artifact

Unity 或其他内网客户端可以上传截图/附件，qamcp 后续通过 `downloadUrl` 下载。上传 body 是 raw bytes，不是 JSON。

```http
POST /api/artifacts?clientId=client-id&kind=screenshot&fileName=screen.png
Content-Type: image/png
```

支持的 `Content-Type` 为 `image/png`、`image/jpeg` 和 `application/octet-stream`。单个文件大小受 `QA_ARTIFACT_MAX_BYTES` 限制，默认 20MB；超出返回 `413 Payload Too Large`。`fileName` 会被服务端清洗后保存，不能进行路径穿越。

返回：

```json
{
  "artifactId": "uuid",
  "kind": "screenshot",
  "clientId": "client-id",
  "fileName": "screen.png",
  "contentType": "image/png",
  "sizeBytes": 12345,
  "sha256": "hex",
  "createdAt": "2026-06-09T10:00:00.000Z",
  "downloadUrl": "/api/artifacts/uuid/download"
}
```

查询元数据：

```http
GET /api/artifacts/:artifactId
```

下载文件：

```http
GET /api/artifacts/:artifactId/download
```

下载响应使用 artifact 保存的 `contentType`，并通过 `Content-Disposition` 使用保存后的 `fileName`。

### 执行单个方法

```http
POST /api/unity-clients/:clientId/execute
Content-Type: application/json

{
  "methodId": "Method(System.String)",
  "methodName": "显示名",
  "arguments": ["arg1"],
  "timeoutMs": 20000,
  "allowParallelExecution": false
}
```

服务端立即返回 `202 Accepted` 和 running 状态。最终结果通过 WebSocket 的 `qa_result` 广播，也会写入 `/api/results`。

PowerShell 示例：

```powershell
curl.exe -X POST http://localhost:3000/api/unity-clients/<clientId>/execute `
  -H "Content-Type: application/json" `
  -d "{\"methodId\":\"<methodId>\",\"methodName\":\"<methodName>\",\"arguments\":[\"arg1\"]}"
```

## WebSocket 协议

WebSocket 路径固定为 `/ws`，用 `role` 查询参数区分连接类型：

- `ws://localhost:3000/ws?role=unity`: Unity 客户端连接入口。
- `ws://localhost:3000/ws?role=web`: Web 控制台或 MCP 工具连接入口。浏览器控制台在配置 `QA_WEB_CONSOLE_TOKEN` 后需要先登录；qamcp 保持原调用方式。

网络断线、Unity 业务 heartbeat 超时、执行超时和控制端占用的目标处理策略见：[WebSocket 状态收敛-断线超时锁与 Late Result](./WebSocket状态收敛-断线超时锁与LateResult.md)。

### Unity 注册

Unity 连接后发送：

```json
{
  "type": "register",
  "clientId": "client-id",
  "name": "Game@Device",
  "ipAddress": "192.168.1.50",
  "ipAddresses": ["192.168.1.50"],
  "platform": "WindowsEditor",
  "unityVersion": "2022.3.62f1",
  "deviceName": "DESKTOP-001",
  "operatingSystem": "Windows 11  (10.0.22631) 64bit",
  "busy": false,
  "currentRequestId": "",
  "currentMethodName": "",
  "methods": [
    {
      "id": "Ping()",
      "name": "连通性检查",
      "declaringType": "Namespace.Type",
      "description": "从 Unity 返回一个简单响应。",
      "returnType": "String",
      "isStatic": true,
      "allowParallelExecution": false,
      "parameters": []
    }
  ]
}
```

服务端返回：

```json
{
  "type": "registered",
  "clientId": "client-id"
}
```

同时向 Web 端广播 `unity_registered`。如果同一个 `clientId` 重新注册，旧连接会被替换。

如果 `name` 与当前已注册的其他 Unity WebSocket 连接重复，服务端会拒绝本次注册、记录 `unity_duplicate_client_name_rejected` 日志，并返回 fatal error 后关闭该 Unity WebSocket：

```json
{
  "type": "error",
  "fatal": true,
  "code": "duplicate_client_name",
  "error": "QaTest 客户端名称“Game@Device”已存在，当前连接已被拒绝。请修改 QaTestClient 的 clientName 后重试。",
  "clientId": "client-id",
  "clientName": "Game@Device",
  "existingClientId": "existing-client-id"
}
```

同一条 Unity WebSocket 连接内刷新注册不视为名称重复；新 WebSocket 连接只要使用了已在线的 `name`，即使 `clientId` 相同，也会被拒绝。

### 心跳和清理

Unity 客户端默认每 10 秒发送：

```json
{
  "type": "heartbeat",
  "clientId": "client-id",
  "busy": true,
  "currentRequestId": "request-id",
  "currentMethodName": "连通性检查"
}
```

服务端返回 `heartbeat_ack`。`busy`、`currentRequestId` 和 `currentMethodName` 表示 Unity 客户端本地执行状态，Web 控制台会把 `clientBusy=true` 的客户端视为不可继续下发请求。服务端每 15 秒检查连接；如果 Unity 客户端超过 45 秒没有更新 `lastSeenAt`，会被标记为 `available=false` 并广播 `unity_unavailable`，但不会立即从在线列表移除。WebSocket close、error 或 ping/pong 失败时，才会广播 `unity_disconnected` 并移除连接。

### Web 快照和事件

Web 端连接后会收到：

```json
{
  "type": "snapshot",
  "clients": [],
  "history": []
}
```

后续可能收到这些事件：

- `unity_registered`: Unity 客户端上线或刷新注册。
- `unity_state_changed`: Unity 客户端本地 busy 状态变化。
- `unity_disconnected`: Unity 客户端断开或过期。
- `unity_unavailable`: Unity WebSocket 仍在线，但业务 heartbeat 超时。
- `unity_available`: Unity 业务 heartbeat 恢复。
- `client_locked`: Unity 客户端被某个 Web/MCP 控制端占用。
- `client_unlocked`: Unity 客户端控制锁释放。
- `execution_started`: 单次执行已下发。
- `execute_accepted`: Web 发起的单次执行已被接受。
- `execute_rejected`: Web 发起的单次执行被拒绝。
- `qa_result`: Unity 回传执行结果。
- `qa_result_late`: 已超时或已取消的请求迟到返回，不覆盖最终历史状态。
- `sequence_started`: 请求序列开始。
- `sequence_step_started`: 请求序列中的单步开始。
- `sequence_step_result`: 请求序列中的单步返回。
- `sequence_finished`: 请求序列结束。
- `stop_accepted`: 停止执行或停止序列请求已接受。
- `stop_rejected`: 停止请求被拒绝。
- `error`: 服务端错误消息。

### Web 执行单个方法

Web 端发送：

```json
{
  "type": "execute",
  "clientId": "client-id",
  "methodId": "Method(System.String)",
  "methodName": "显示名",
  "arguments": ["arg1"],
  "timeoutMs": 20000,
  "allowParallelExecution": false
}
```

服务端会给目标 Unity 发送。`allowParallelExecution` 只有在请求显式传入 `true`，或 Unity 注册的方法元数据包含 `allowParallelExecution: true` 时才会为 `true`：

```json
{
  "type": "execute",
  "requestId": "uuid",
  "methodId": "Method(System.String)",
  "methodName": "显示名",
  "allowParallelExecution": false,
  "arguments": ["arg1"]
}
```

Unity 完成后返回 `qa_result`。Unity client 会把 `failed:<reason>`、`false`、`ok=false` 或失败态 `status` 的结构化结果映射为 `success=false`；server 对同类结果也会做兜底判定：

```json
{
  "type": "qa_result",
  "requestId": "uuid",
  "clientId": "client-id",
  "methodId": "Method(System.String)",
  "methodName": "显示名",
  "success": true,
  "result": "pong",
  "error": "",
  "durationMs": 12
}
```

服务端会补充 `status`、`finishedAt` 等字段后广播给 Web 端，并写入历史。

当 Unity 注册的方法元数据包含 `allowParallelExecution: true`，或执行请求显式传入 `allowParallelExecution: true` 时，Rust server 会允许同一 `clientId` 的该请求与其它执行并行下发，并把该字段透传给 Unity。该能力应只用于只读查询类方法；默认仍保持同一 Unity 客户端互斥执行。

## 请求序列

Web 端或 MCP 工具可以发送 `execute_sequence`：

```json
{
  "type": "execute_sequence",
  "sequenceId": "uuid",
  "clientId": "client-id",
  "stopOnFailure": true,
  "stepDelayMs": 1000,
  "steps": [
    {
      "stepId": "step-1",
      "methodId": "Ping()",
      "methodName": "连通性检查",
      "arguments": []
    }
  ]
}
```

字段说明：

- `sequenceId`: 可选。不传时由服务端生成。
- `clientId`: 必填，目标 Unity 客户端。
- `steps`: 必填，至少一个有效步骤。每步需要 `methodId` 或 `methodName`；无效 step 当前会被过滤，如果过滤后为空则拒绝请求。
- `stopOnFailure`: 默认 `true`。步骤失败后停止后续步骤。
- `stepDelayMs`: 相邻步骤之间的等待时间，单位毫秒，范围 `0` 到 `300000`。
- `timeoutMs`: 可选，作为所有 step 的默认执行超时；单个 step 的 `timeoutMs` 优先级更高。

服务端会按顺序下发每一步，并等待 Unity 返回后再进入下一步。单步等待结果的默认超时时间复用 `EXECUTION_TIMEOUT_MS`，默认 20000 毫秒；序列顶层或单个 step 可通过 `timeoutMs` 覆盖。超时会生成 failed 结果。

## 历史记录

服务端在内存中保留最近 200 条执行记录。记录包含：

- `requestId`
- `clientId`
- `methodId`
- `methodName`
- `status`: `running`、`success`、`failed` 或 `cancelled`
- `success`
- `result`
- `error`
- `durationMs`
- `startedAt` 或 `finishedAt`
- 请求序列相关字段：`sequenceId`、`stepId`、`stepIndex`、`stepNumber`、`totalSteps`

服务重启后历史记录会清空。

## 校验

文档或代码变更后建议运行：

```powershell
cd registerserver/rustserver
cargo fmt
cargo check
cargo test
```

```powershell
cd ../client
npm run build
```

Node legacy 已冻结，不参与常规开发和发布验证。只有确实需要临时回退运行时才检查：

```powershell
cd ../server
node --check src/index.js
```

Node legacy 的定位、风险和回退步骤见：[NodeLegacy 服务端-冻结规则与回退指南](./NodeLegacy服务端-冻结规则与回退指南.md)。

当前仓库没有统一测试脚本。

## 常见问题

### 页面显示没有 Unity 实例

先确认服务端已启动，再确认 Unity 已进入 Play Mode、QA 功能已启用，并且 Unity 控制台出现 `[QaTest] Connected.` 日志。默认 Unity 连接地址是：

```text
ws://localhost:3000/ws?role=unity
```

如果服务端不在本机或端口不是 `3000`，需要在 Unity 客户端覆盖连接地址。构建后的 Player 默认关闭 QA 功能，需要通过 `--qa-test-enable`、`QA_TEST_ENABLED=1`、PlayerPrefs `QaTest.Enabled=1` 或 `QaTestClient.SetGlobalEnabled(true)` 开启；`--qa-enable` 仍作为旧别名兼容。

### 方法列表为空

确认目标方法已经添加 `[QaTest]`，并且满足 Unity 客户端扫描规则：静态方法可以直接注册；实例方法必须定义在场景中已存在的 `MonoBehaviour` 实例上。

### 请求一直 running

服务端单次 HTTP 执行只表示“已下发”，结果依赖 Unity 通过 WebSocket 回传。检查 Unity 是否仍在线、方法内部是否阻塞、协程或 Task 是否完成。请求序列中的单步等待超时复用 `EXECUTION_TIMEOUT_MS`，默认 20000 毫秒。

### 前端开发服务器无法调用 API

开发模式下需要同时启动 `registerserver/rustserver`。`registerserver/client/vite.config.js` 已配置把 `/api` 和 `/ws` 代理到 `http://localhost:3000`。
