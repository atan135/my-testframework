# WebSocket 状态收敛：断线、超时、锁与 Late Result

本文档记录 QA register server 对 Unity 客户端、Web 控制台和 MCP 控制端的 WebSocket 故障处理策略。当前 server、Web 控制台和 qamcp 已按本文档实现核心策略。

## 目标

在弱网、断网、Unity 卡顿、控制端刷新或断线等情况下，server 需要保证：

- 执行请求最终收敛到明确状态，不长期停留在 `running`。
- Unity 连接状态、业务可用状态和控制占用状态分开表达。
- 控制端断线不影响已提交请求继续执行。
- 同一 Unity 设备同一时间只允许一个 Web/MCP 控制端控制。
- WebSocket 协议层故障和 Unity 业务心跳故障分别处理。

## 状态定义

server 侧把 Unity 设备状态拆成四类：

```text
online / offline
available / unavailable
locked / unlocked
running / idle
```

### online / offline

表示 WebSocket 连接是否存在并且协议层仍然健康。

- `online`: Unity WebSocket 连接存在，server ping 能收到 pong。
- `offline`: WebSocket close、error、ping/pong 失败或 socket 被 terminate。

### available / unavailable

表示 Unity 业务层是否可执行测试。

- `available`: 最近收到 Unity `register` 或 `heartbeat`。
- `unavailable`: WebSocket 还在，但 Unity 业务 heartbeat 超时。

业务 heartbeat 超时后不删除连接，只标记不可用，等待后续 `heartbeat` 或 `register` 恢复。

### locked / unlocked

表示 Unity 设备是否被某个控制端占用。

- `locked`: 已被某个 Web/MCP 控制端占用。
- `unlocked`: 当前没有控制端占用。

同一 Unity 设备同一时间只允许一个控制端控制。如果设备已被一个 Web/MCP 控制端拉起，其他控制端不能再控制该设备。

例外：只读查询类 `[QaTest]` 方法可以通过注册元数据或执行请求声明 `allowParallelExecution=true`。这类请求不会按普通锁、running 和 Unity 本地 busy 状态做互斥拦截，但仍会生成独立 `requestId`、写入历史并等待结果。

### running / idle

表示设备上是否存在 server 已下发但未完成的执行。

- `running`: 有单次执行或请求序列正在等待结果。
- `idle`: 没有进行中的执行。

## 配置项

使用环境变量配置关键时间：

```powershell
$env:EXECUTION_TIMEOUT_MS=20000
$env:UNITY_HEARTBEAT_STALE_MS=45000
```

默认值：

- `EXECUTION_TIMEOUT_MS`: `20000`，执行请求超时时间，单位毫秒。
- `UNITY_HEARTBEAT_STALE_MS`: `45000`，Unity 业务 heartbeat 超时时间，单位毫秒。
- WebSocket ping 间隔可继续保持 `15000` 毫秒。

Unity 当前默认每 10 秒发送一次业务 heartbeat。`UNITY_HEARTBEAT_STALE_MS=45000` 可以容忍短暂卡顿，同时不会让不可用设备长期显示为可执行。

## WebSocket 协议层故障

server 定期对所有 WebSocket 连接发送 ping。

如果某个 socket 没有按期返回 pong：

1. server terminate 该 socket。
2. 如果是 Unity socket，标记设备 `offline`。
3. 广播 `unity_disconnected` 或等价事件。
4. 该设备不再接受新的执行请求。
5. 已经 running 的执行不立即消失，按 `EXECUTION_TIMEOUT_MS` 超时收敛为 failed。

WebSocket close 或 error 与 ping/pong 失败一致处理。

## Unity 业务 Heartbeat 超时

Unity 业务 heartbeat 表示 Unity 主循环和 QaTestClient 业务逻辑仍在工作。它和 WebSocket ping/pong 不是同一个概念。

当 Unity WebSocket 仍然在线，但超过 `UNITY_HEARTBEAT_STALE_MS` 没有收到业务 heartbeat：

1. 设备保持 `online`。
2. 标记 `available=false`。
3. 广播设备不可用事件或在快照中体现不可用状态。
4. 不接受新的执行请求。
5. 已经 running 的执行按 `EXECUTION_TIMEOUT_MS` 超时收敛。
6. 后续收到 `heartbeat`、`register` 或其他 Unity 业务消息后，标记 `available=true` 并广播恢复。

该策略避免因为临时业务卡顿直接踢掉 socket，也能避免把业务不可用的设备继续暴露为可执行。

## 执行超时

所有执行请求都需要超时控制，包括：

- Web 控制台发起的单次执行。
- MCP 发起的单次执行。
- 请求序列中的每一步。

请求下发后进入 `running`，server 创建 pending 记录并启动计时器。默认 20 秒内没有收到 Unity `qa_result`，server 生成 failed 结果：

```json
{
  "status": "failed",
  "success": false,
  "error": "Timed out after 20000 ms."
}
```

超时结果需要：

- 写入执行历史。
- 广播给当前可用的 Web/MCP 观察端。
- 如果属于请求序列，驱动序列进入下一步或按 `stopOnFailure` 停止。

如果超时后 Unity 又返回同一个 `requestId` 的 `qa_result`，建议忽略或记录为 late result，不再覆盖已经失败的最终状态。

## 控制端断线

Web 控制台或 MCP 控制端断线后，server 不取消已经提交的执行。

目标行为：

1. 已下发的单次执行继续等待 Unity 返回或超时。
2. 已提交的请求序列继续执行到完成、失败、超时或被主动停止。
3. 控制端断线期间产生的执行结果仍写入 server 历史。
4. 控制端重连后可以通过历史或快照看到最终执行结果。

控制端断线不等于 stop。只有控制端主动发送停止指令时，server 才停止后续序列步骤。

## 控制锁策略

每个 Unity 设备最多有一个控制端 owner。

控制端可以是：

- Web 控制台连接。
- MCP 连接。

锁信息包含：

```json
{
  "ownerId": "web-or-mcp-session-id",
  "ownerType": "web",
  "acquiredAt": "2026-04-28T08:00:00.000Z",
  "lastSeenAt": "2026-04-28T08:00:10.000Z"
}
```

`clientId` 由 `client_locked`、`client_unlocked` 事件外层字段携带，不在 `lock` 对象内部重复存储。

### 获取锁

控制端对 Unity 发起执行或请求序列前，需要先获得该 Unity 的控制锁。

如果设备未被占用：

- server 授予锁。
- 请求继续执行。

如果设备已被其他控制端占用：

- server 拒绝请求。
- 返回明确错误，例如 `Unity client is locked by another controller.`

### 锁释放

策略确认如下：

- 控制端断线但仍有 running 请求或序列：锁保留到执行结束或超时。
- 控制端断线且没有 running 请求：立即释放锁。
- 控制端主动 stop/cancel 后：停止后续序列步骤；如果没有 running 请求，释放锁。
- Unity WebSocket 离线：执行按超时收敛；锁在执行结束或超时后释放。
- Unity 只是业务 heartbeat 超时：不立即释放锁，等待执行超时或恢复。

## 主动停止

server 应支持 Web/MCP 主动发送停止指令。

当前语义：

- 对尚未下发的序列步骤：停止执行。
- 对已经下发到 Unity 的单个方法：server 侧标记取消或等待其返回。
- 如果 Unity 端暂时没有取消执行机制，server 不能强制中断 Unity 中正在运行的方法。
- 已取消的请求需要有明确状态，例如 `cancelled`，不要停留在 `running`。

请求序列被停止后，应广播 `sequence_finished`，状态可为 `cancelled`。

## 新执行请求校验

server 接受执行请求前需要检查：

1. `clientId` 存在。
2. Unity socket `online=true`。
3. Unity `available=true`。
4. 设备未被其他控制端锁定。
5. Unity 本地没有上报 `clientBusy=true`。
6. 请求包含有效 `methodId`。

任何一项不满足，都应立即拒绝，不进入 running。声明 `allowParallelExecution=true` 的只读查询类请求会放宽第 4、5 项以及普通 running 互斥，但不应承担会修改 Unity 状态的操作。

典型错误：

- `Unity client is not online.`
- `Unity client is unavailable.`
- `Unity client is locked by another controller.`
- `Unity client is already busy.`
- `methodId is required.`

## 重连替换

同一个 Unity `clientId` 重新注册时，server 应替换旧连接。

如果新注册连接的 `clientName` 与已在线的其他 Unity WebSocket 连接重复，server 不做替换，而是拒绝新连接并返回 fatal `duplicate_client_name` error；Unity 客户端收到后停止重连并退出启动流程。

实现时需要避免旧 socket 的 `close` 回调误删新连接：

- 删除 Unity 客户端时校验 socket identity。
- 只有当前 map 中保存的 socket 与触发 close/error 的 socket 相同，才删除该客户端。

目标行为：

1. 新连接 register。
2. server 更新 `unityClients[clientId]` 到新 socket。
3. server 关闭旧 socket。
4. 旧 socket close 事件到达时，不影响新 socket。

## 事件建议

为方便 Web 控制台和 MCP 观察，server 会广播以下事件：

- `unity_registered`: Unity 注册或重新注册。
- `unity_state_changed`: Unity 本地 busy 状态变化。
- `unity_disconnected`: Unity WebSocket 离线。
- `unity_unavailable`: Unity 业务 heartbeat 超时。
- `unity_available`: Unity 业务 heartbeat 恢复。
- `client_locked`: 设备被控制端占用。
- `client_unlocked`: 设备控制锁释放。
- `execution_started`: 执行开始。
- `qa_result`: 执行完成、失败、超时或取消。
- `qa_result_late`: 已超时或已取消请求的迟到结果。
- `sequence_started`: 请求序列开始。
- `sequence_step_started`: 序列步骤开始。
- `sequence_step_result`: 序列步骤结束。
- `sequence_finished`: 请求序列结束。
- `stop_accepted`: 停止请求已接受。
- `stop_rejected`: 停止请求被拒绝。

## 验收场景

### Unity 网络断开

步骤：

1. Unity 已注册。
2. 发起一个执行时间较长的方法。
3. 执行中断开 Unity 网络或关闭 Play Mode。

期望：

- WebSocket 最终离线。
- 新执行请求被拒绝。
- running 请求在 20 秒左右变为 failed。
- 历史记录包含超时错误。

### Unity 主循环卡住但 socket 未断

步骤：

1. Unity 已注册。
2. 让 Unity 不再发送业务 heartbeat，但 WebSocket 连接未立即断开。

期望：

- 设备标记为 unavailable。
- 新执行请求被拒绝。
- socket 不被立即关闭。
- 后续 heartbeat 恢复后设备重新 available。

### 控制端断线

步骤：

1. Web/MCP 发起请求序列。
2. 序列执行中断开 Web/MCP 控制端连接。

期望：

- server 继续执行序列。
- 最终结果写入历史。
- 锁保留到序列结束或超时。

### 多控制端竞争

步骤：

1. 控制端 A 锁定 Unity 并发起执行。
2. 控制端 B 对同一 Unity 发起执行。

期望：

- B 被拒绝。
- A 的执行不受影响。
- A 结束后锁按策略释放。

### 同 clientId 重连

步骤：

1. Unity A 注册 `clientId=x`。
2. Unity A 网络抖动后重新连接并使用同一 `clientId=x` 注册。

期望：

- 新连接替换旧连接。
- 旧连接 close 事件不会删除新连接。
- Web 控制台最终只显示一个在线 Unity 实例。

## 实现清单

当前实现已覆盖：

1. `EXECUTION_TIMEOUT_MS` 环境变量配置，默认 20000。
2. 单次执行和请求序列步骤统一进入 pending execution。
3. pending execution 超时后写入 failed 历史并广播。
4. Unity `available` 状态；heartbeat stale 只标记不可用，不删除客户端。
5. 收到 heartbeat、register 或其他 Unity 业务消息时恢复 `available`。
6. 控制端 session id 和 Unity 控制锁。
7. 执行请求前校验 online、available、lock owner 和 running 状态。
8. 控制端断线时按 running 状态决定是否释放锁。
9. `stop_sequence`、`cancel_sequence`、`stop_execution`、`cancel_execution` 消息。
10. 同 clientId 重连时校验 socket identity，避免旧 socket close 误删新 socket。
11. Unity 本地 busy 状态纳入 running 判断和普通执行请求拦截。
12. `allowParallelExecution` 只读查询类请求可绕过普通互斥，并由 server 透传给 Unity client。
