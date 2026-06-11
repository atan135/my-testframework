# RustServer：后续优化 Todo

本文从 Rust register server 的请求处理流程、状态收敛和性能分析整理后续任务。具体接口和协议细节仍以对应专题文档为准；本文只作为执行优先级和工程取舍入口。

## P0：正确性和验收闭环

- [ ] 建立最小协议集成测试，覆盖 Unity `register`、Web `snapshot`、单次 `execute`、`qa_result`、执行超时、`stop_execution`、`execute_sequence`、`sequence_finished` 和 late result。
- [ ] 明确停止语义：server 侧取消后，Unity 仍可能继续执行并迟到回包；需要补充协议说明、前端展示和测试覆盖，避免误以为 Unity 方法已被强制中断。
- [ ] 对请求序列 step 做严格校验，避免 `normalize_sequence_steps` 静默过滤错误输入；校验失败时返回具体 step index、缺失字段和参数类型错误。
- [ ] 修复并验证 Web 控制台 sequence cancelled 统计和 step 结果合并优先级，确保 UI、server 事件和 history 表示一致。

## P1：可观测性和性能

- [ ] 增加 server 指标出口，建议先提供 `/api/metrics` 或 Prometheus `/metrics`，覆盖在线 Unity 数、Web/MCP 连接数、pending executions、active sequences、timeout、cancel、late result、WS outbound queue full。
- [ ] 引入请求级 tracing/span，把 `requestId`、`sequenceId`、`clientId`、`controllerId` 串到 HTTP、WebSocket、执行调度、结果收敛和 archive 日志中。
- [ ] 优化 WebSocket 广播路径：锁内只收集 Web sender 和必要快照，锁外完成 JSON 序列化与 `try_send`，降低 `ServerState` 全局写锁持有时间。
- [ ] 优化 archive 入队路径：锁内只采集必要快照，锁外构造 archive JSON record，避免方法元数据扫描和大 result 序列化占用状态锁。
- [ ] 对 outbound queue full 建立显式计数和日志采样；当前 `try_send` 失败多数只丢弃广播，需要有指标判断慢消费者影响。

## P1：协议契约和测试资产

- [ ] 抽出稳定协议契约，使用 JSON Schema、OpenAPI 示例集或 `schemars` 生成方式描述 HTTP body、WebSocket event 和 `ExecutionRecord`。
- [ ] 为 Rust server 补充行为测试，覆盖锁竞争、Unity heartbeat stale、ping timeout、controller 断线后锁保留/释放、并行执行例外。
- [ ] 为 qamcp 补充 mock register server 集成测试，确保 Agent 工具等待超时、事件监听和 server 超时语义不会互相覆盖。

## P2：安全和多用户能力

- [ ] 补齐全局 HTTP/WebSocket 鉴权，区分 Web 控制台登录 token、Unity client token 和 qamcp/controller token。
- [ ] 增加角色权限模型：viewer 只能看状态和历史，executor 可以执行，admin 可以停止、踢连接、调整运行参数。
- [ ] 支持细粒度白名单或部署在反向代理后的可信 header 校验，降低非可信网络暴露风险。

## P2：测试产物和历史聚合

- [ ] 完善结构化测试结果协议，减少 `qa_result.result` 仅靠字符串承载业务状态。
- [ ] 支持测试结果附件，例如截图、Unity log、性能指标和大结果文件，并定义大小限制与存储后端。
- [ ] 支持按测试用例、Agent session、构建版本、设备信息聚合执行历史。
- [ ] 增加 archive 查询 API，例如按时间、状态、client、method、sequence 查询长期历史。

## 技术选型建议

- [ ] 评估 `tokio-util::sync::CancellationToken`，用于序列取消、超时任务和未来 Unity cooperative cancellation 语义。
- [ ] 评估 `tracing` + `tracing-subscriber` 替代手写 JSON line 日志，保留结构化输出并支持 span。
- [ ] 评估 `metrics` + `metrics-exporter-prometheus`，作为最小侵入的指标导出方案。
- [ ] 评估 `schemars` 或 `utoipa`，用于生成协议契约和示例，降低 Web、qamcp、Unity 多端字段漂移。
- [ ] 暂不优先引入 `DashMap` 或 gRPC；当前状态机强一致和现有 HTTP/WebSocket 协议兼容性更重要。

