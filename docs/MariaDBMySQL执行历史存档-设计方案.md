# MariaDB/MySQL 执行历史存档：设计方案

本文设计 register server 的正式执行历史存档能力。目标是在不改变当前 Web 控制台、HTTP API、WebSocket 协议和执行调度主逻辑的前提下，增加一层可选 MariaDB/MySQL 持久化写入，用于长期查询、审计和问题追溯。

当前本地部署目标数据库为 **MariaDB 10.5**。文档中的环境变量仍使用 `MYSQL` 命名，是因为 MariaDB 兼容 MySQL 协议和常用 Rust MySQL 驱动连接方式。

如果只需要测试环境或线上环境部署步骤，直接看 [MariaDB 执行历史存档-部署与验证手册](./MariaDB执行历史存档-部署与验证手册.md)。

## 目标

- 通过 `.env` 增加数据库功能开关，默认关闭。
- 开启后，Rust server 启动时连接 MariaDB/MySQL。
- 执行请求和执行结果继续按现有内存 history、WebSocket 广播和 Web 页面逻辑运行。
- 额外把执行记录保存到 MariaDB/MySQL，供长期追溯。
- 数据库写入失败不能影响现有执行链路，最多记录错误日志。
- 当前阶段不修改 Web 查询页面、不新增历史查询 API、不改变 Unity/qamcp 协议。

## 非目标

- 不用 MariaDB/MySQL 替换当前 `ServerState.execution_history`。
- 不从 MariaDB/MySQL 回放 `/api/results`。
- 不改变 Web 控制台“最近结果”的实时显示方式。
- 不增加用户权限系统。当前只能记录 `controllerId/controllerType`，无法严格等同真实操作人。
- 不强依赖数据库可用性。数据库不可用时，server 应按当前逻辑继续运行。

## 总体方案

保持现有主链路不变：

```text
Web/qamcp 发起执行
  -> register server 校验、加锁、下发 Unity
  -> 内存 history 记录 running
  -> WebSocket 广播 execution_started
  -> Unity 返回 qa_result 或 server 超时/取消
  -> 内存 history 更新最终状态
  -> WebSocket 广播 qa_result
```

新增旁路存档链路：

```text
内存 history 写入或更新后
  -> 构造 ExecutionArchiveRecord
  -> 发送到异步归档队列
  -> 后台任务 upsert MariaDB/MySQL
```

关键原则：

- MariaDB/MySQL 只做旁路写入，不参与执行调度决策。
- 不在 `ServerState` 锁内执行数据库 IO。
- 写入使用 upsert，以 `request_id` 保证同一执行从 `running` 更新到最终状态。
- 队列满、数据库断开或写入失败时记录 JSON line 日志，不阻塞执行。

## 配置设计

新增 `.env` 配置项：

```env
# 是否启用 MariaDB/MySQL 执行历史存档。
# false: 默认关闭，不连接数据库。
# true: 启动时初始化 MariaDB/MySQL 连接池，并异步保存执行历史。
QA_EXECUTION_ARCHIVE_MYSQL_ENABLED=false

# MariaDB/MySQL 连接串。
# 示例：mysql://qa_register:qa_register_password@127.0.0.1:3306/qa_register_archive
QA_EXECUTION_ARCHIVE_MYSQL_URL=

# 连接池最大连接数。
QA_EXECUTION_ARCHIVE_MYSQL_MAX_CONNECTIONS=5

# 异步写入队列长度。队列满时丢弃存档任务并记录 warn 日志，不能阻塞执行。
QA_EXECUTION_ARCHIVE_QUEUE_SIZE=10000
```

启动行为：

- `QA_EXECUTION_ARCHIVE_MYSQL_ENABLED` 未设置或为 `false`：完全不初始化 MariaDB/MySQL。
- 设置为 `true` 但 `QA_EXECUTION_ARCHIVE_MYSQL_URL` 为空：启动时记录 error，并禁用归档层；主服务继续启动。
- 数据库初始化失败：记录 error，并禁用归档层；主服务继续启动。
- 运行中写入失败：记录 error，可做有限重试；不影响执行结果返回。

## 数据模型

### 记录粒度

一条 `request_id` 对应一条执行记录。

执行生命周期：

- 请求被接受后写入 `running`。
- 成功、失败、取消、超时后通过同一个 `request_id` upsert 为最终状态。
- 如果是请求序列，每个 step 也有独立 `request_id`，并携带 `sequence_id/step_id/step_index`。

### 字段设计

主表保存可检索字段和完整 JSON：

- 可检索字段：时间、状态、client、controller、method、sequence。
- 完整 JSON 字段：arguments、result、method metadata、client snapshot、原始 execution record。
- 大字段不参与索引，避免影响写入和查询性能。

当前没有真实用户系统，因此先记录：

- `controller_id`
- `controller_type`

后续如果 Web 登录改成真实用户，可新增：

- `operator_id`
- `operator_name`

## MariaDB 10.5 建库建表 SQL

以下 SQL 面向 MariaDB 10.5。本地部署时可先用 root 用户执行建库和授权，再用业务用户连接。

兼容说明：

- MariaDB 10.5 不支持 MySQL 8.0 的 `utf8mb4_0900_ai_ci` 排序规则，本文统一使用 `utf8mb4_unicode_ci`。
- MariaDB 的 `JSON` 类型是兼容类型，底层按文本保存并校验 JSON。写入时直接传 JSON 字符串，不使用 MySQL 8 风格的 `CAST(? AS JSON)`。
- `ON DUPLICATE KEY UPDATE ... VALUES(column)` 在 MariaDB 10.5 可用。

### 创建数据库和用户

```sql
CREATE DATABASE IF NOT EXISTS qa_register_archive
  DEFAULT CHARACTER SET utf8mb4
  DEFAULT COLLATE utf8mb4_unicode_ci;

CREATE USER IF NOT EXISTS 'qa_register'@'localhost'
  IDENTIFIED BY 'qa_register_password';

CREATE USER IF NOT EXISTS 'qa_register'@'127.0.0.1'
  IDENTIFIED BY 'qa_register_password';

GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX
  ON qa_register_archive.*
  TO 'qa_register'@'localhost';

GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX
  ON qa_register_archive.*
  TO 'qa_register'@'127.0.0.1';

FLUSH PRIVILEGES;
```

本地 `.env` 连接串示例：

```env
QA_EXECUTION_ARCHIVE_MYSQL_ENABLED=true
QA_EXECUTION_ARCHIVE_MYSQL_URL=mysql://qa_register:qa_register_password@127.0.0.1:3306/qa_register_archive
```

### 创建迁移记录表

```sql
USE qa_register_archive;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version VARCHAR(64) NOT NULL,
  description VARCHAR(255) NOT NULL,
  applied_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
  PRIMARY KEY (version)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### 创建执行记录表

```sql
USE qa_register_archive;

CREATE TABLE IF NOT EXISTS execution_records (
  id BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,

  request_id VARCHAR(64) NOT NULL,
  sequence_id VARCHAR(64) NULL,
  step_id VARCHAR(64) NULL,
  step_index INT UNSIGNED NULL,
  step_number INT UNSIGNED NULL,
  total_steps INT UNSIGNED NULL,

  client_id VARCHAR(128) NOT NULL,
  client_name VARCHAR(255) NULL,
  client_ip VARCHAR(64) NULL,
  client_platform VARCHAR(64) NULL,
  unity_version VARCHAR(64) NULL,

  controller_id VARCHAR(128) NULL,
  controller_type VARCHAR(64) NULL,

  method_id VARCHAR(512) NOT NULL,
  method_name VARCHAR(255) NULL,
  method_real_name VARCHAR(255) NULL,
  method_display_name VARCHAR(255) NULL,
  declaring_type VARCHAR(512) NULL,
  return_type VARCHAR(255) NULL,
  allow_parallel_execution BOOLEAN NOT NULL DEFAULT FALSE,

  status VARCHAR(32) NOT NULL,
  success BOOLEAN NULL,
  duration_ms BIGINT UNSIGNED NULL,
  error_text MEDIUMTEXT NULL,

  arguments_json JSON NULL,
  result_json JSON NULL,
  method_metadata_json JSON NULL,
  client_snapshot_json JSON NULL,
  execution_record_json JSON NOT NULL,

  started_at DATETIME(3) NULL,
  finished_at DATETIME(3) NULL,
  created_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
  updated_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3),

  PRIMARY KEY (id),
  UNIQUE KEY uk_execution_records_request_id (request_id),
  KEY idx_execution_records_started_at (started_at),
  KEY idx_execution_records_finished_at (finished_at),
  KEY idx_execution_records_client_started (client_id, started_at),
  KEY idx_execution_records_controller_started (controller_id, started_at),
  KEY idx_execution_records_method_started (method_id(191), started_at),
  KEY idx_execution_records_status_started (status, started_at),
  KEY idx_execution_records_sequence (sequence_id, step_index),
  KEY idx_execution_records_updated_at (updated_at),

  CONSTRAINT chk_execution_records_status
    CHECK (status IN ('running', 'success', 'failed', 'cancelled'))
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

### 记录 schema 版本

```sql
USE qa_register_archive;

INSERT INTO schema_migrations (version, description)
VALUES ('20260519_001', 'create execution archive tables')
ON DUPLICATE KEY UPDATE description = VALUES(description);
```

## Upsert SQL 设计

归档 writer 对每条执行记录使用 `request_id` upsert：

```sql
INSERT INTO execution_records (
  request_id,
  sequence_id,
  step_id,
  step_index,
  step_number,
  total_steps,
  client_id,
  client_name,
  client_ip,
  client_platform,
  unity_version,
  controller_id,
  controller_type,
  method_id,
  method_name,
  method_real_name,
  method_display_name,
  declaring_type,
  return_type,
  allow_parallel_execution,
  status,
  success,
  duration_ms,
  error_text,
  arguments_json,
  result_json,
  method_metadata_json,
  client_snapshot_json,
  execution_record_json,
  started_at,
  finished_at
) VALUES (
  ?, ?, ?, ?, ?, ?,
  ?, ?, ?, ?, ?,
  ?, ?,
  ?, ?, ?, ?, ?, ?, ?,
  ?, ?, ?, ?,
  ?,
  ?,
  ?,
  ?,
  ?,
  ?, ?
)
ON DUPLICATE KEY UPDATE
  sequence_id = VALUES(sequence_id),
  step_id = VALUES(step_id),
  step_index = VALUES(step_index),
  step_number = VALUES(step_number),
  total_steps = VALUES(total_steps),
  client_id = VALUES(client_id),
  client_name = VALUES(client_name),
  client_ip = VALUES(client_ip),
  client_platform = VALUES(client_platform),
  unity_version = VALUES(unity_version),
  controller_id = VALUES(controller_id),
  controller_type = VALUES(controller_type),
  method_id = VALUES(method_id),
  method_name = VALUES(method_name),
  method_real_name = VALUES(method_real_name),
  method_display_name = VALUES(method_display_name),
  declaring_type = VALUES(declaring_type),
  return_type = VALUES(return_type),
  allow_parallel_execution = VALUES(allow_parallel_execution),
  status = VALUES(status),
  success = VALUES(success),
  duration_ms = VALUES(duration_ms),
  error_text = VALUES(error_text),
  arguments_json = VALUES(arguments_json),
  result_json = VALUES(result_json),
  method_metadata_json = VALUES(method_metadata_json),
  client_snapshot_json = VALUES(client_snapshot_json),
  execution_record_json = VALUES(execution_record_json),
  started_at = COALESCE(execution_records.started_at, VALUES(started_at)),
  finished_at = VALUES(finished_at),
  updated_at = CURRENT_TIMESTAMP(3);
```

说明：

- `started_at` 首次 running 写入后不应被后续空值覆盖。
- `finished_at` 可由最终结果覆盖。
- `arguments_json`、`result_json`、`method_metadata_json`、`client_snapshot_json` 和 `execution_record_json` 占位符传入合法 JSON 字符串；MariaDB 10.5 不需要也不支持在这里使用 `CAST(? AS JSON)`。
- `execution_record_json` 保存完整 server 内部 `ExecutionRecord` 快照，方便后续兼容字段变化。
- `method_metadata_json` 保存执行时方法元数据快照，避免 Unity 后续改名后老记录失去上下文。
- `client_snapshot_json` 保存执行时客户端名称、IP、平台、Unity 版本等上下文。

## 本地部署步骤

1. 安装并启动 MariaDB 10.5。

2. 用 root 登录：

```powershell
mariadb -u root -p
```

如果本机只安装了 `mysql` 兼容客户端，也可以继续使用 `mysql -u root -p`。

3. 执行本文“创建数据库和用户”“创建迁移记录表”“创建执行记录表”中的 SQL。

4. 编辑 `registerserver/rustserver/.env`：

```env
QA_EXECUTION_ARCHIVE_MYSQL_ENABLED=true
QA_EXECUTION_ARCHIVE_MYSQL_URL=mysql://qa_register:qa_register_password@127.0.0.1:3306/qa_register_archive
QA_EXECUTION_ARCHIVE_MYSQL_MAX_CONNECTIONS=5
QA_EXECUTION_ARCHIVE_QUEUE_SIZE=10000
```

5. 启动 Rust server：

```powershell
cd registerserver/rustserver
cargo run
```

6. 执行一次 QaTest 方法后，用 SQL 验证：

```sql
USE qa_register_archive;

SELECT
  request_id,
  client_name,
  controller_type,
  method_real_name,
  method_display_name,
  status,
  success,
  duration_ms,
  started_at,
  finished_at
FROM execution_records
ORDER BY id DESC
LIMIT 20;
```

## 当前实现

### 模块拆分

当前存档实现位于 `registerserver/rustserver/src/archive/mod.rs`，由 `ExecutionArchive` 统一封装连接池、异步队列、记录快照和 upsert SQL。后续如果存档逻辑继续膨胀，可再按后端拆分子模块。

```text
registerserver/rustserver/src/
  archive/
    mod.rs
```

职责：

- `archive::ExecutionArchive`: 对主执行流程只暴露轻量入队接口。
- 当前 `archive/mod.rs`: 管理连接池、建连检查、异步写入循环和 upsert SQL。
- 主执行流程只在 history 更新后调用存档入队接口。

### 写入触发点

当前在现有内存 history 更新后触发：

- `execution_started` 产生 running 记录后。
- `complete_pending_execution` 产生最终结果后。
- unexpected result 写入 history 后。
- sequence step rejected 生成失败记录后。

存档队列只做快照构造和 `try_send`，真正数据库 IO 在后台 writer 任务中完成；因此不会让主执行链路等待数据库写入。

### 数据补全

`ExecutionRecord` 当前不包含所有归档字段。归档层可以在 enqueue 前从 `ServerState` 中补充快照：

- 根据 `client_id` 找 Unity client，补 `client_name/client_ip/platform/unity_version`。
- 根据 `method_id/method_name` 找注册方法，补 `declaring_type/return_type/description/parameters`。
- 根据 controller 上下文补 `controller_id/controller_type`。

如果某些字段取不到，允许为空，不影响写入。

## 查询预留

当前阶段不改 Web。后续如果需要长期查询页面，建议新增 API：

- `GET /api/archive/executions`
- `GET /api/archive/executions/:requestId`

列表接口只返回摘要字段，不返回完整 `result_json`：

- requestId
- clientName/clientId
- controllerId/controllerType
- methodRealName/methodDisplayName
- status/success
- durationMs
- startedAt/finishedAt

详情接口再返回完整：

- argumentsJson
- resultJson
- errorText
- methodMetadataJson
- clientSnapshotJson

## 风险和处理

| 风险 | 处理 |
| --- | --- |
| MariaDB/MySQL 不可用导致执行失败 | 禁止让 DB 写入阻塞主链路；初始化失败时禁用归档。 |
| 队列堆积占用内存 | 设置 `QA_EXECUTION_ARCHIVE_QUEUE_SIZE`；队列满时丢弃归档任务并打 warn。 |
| result 很大 | `result_json` 使用 JSON；必要时后续可拆附件表或限制大小。 |
| 用户身份不明确 | 当前记录 controller；后续接入真实登录用户后再补 operator 字段。 |
| schema 演进 | 使用 `schema_migrations` 表记录版本，后续 SQL 迁移追加版本。 |
| 敏感数据入库 | `.env` 不提交；result/arguments 可能含账号或 token，生产部署需要控制数据库访问权限和备份权限。 |

## 验收标准

- 默认关闭时，server 行为和当前完全一致，不尝试连接 MariaDB/MySQL。
- 开启但 MariaDB/MySQL 不可用时，server 可以启动并执行 QaTest，请求结果仍正常返回。
- 开启且 MariaDB/MySQL 可用时，每次执行至少写入一条 `execution_records`。
- 同一 `request_id` 从 running 更新为最终状态，不产生重复业务记录。
- 请求序列每个 step 都能按 `sequence_id` 关联查询。
- `npm run build`、`cargo check` 仍通过。
