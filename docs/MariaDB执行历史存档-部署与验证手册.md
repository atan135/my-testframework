# MariaDB 执行历史存档：部署与验证手册

本文用于在测试环境或线上环境初始化 register server 的 MariaDB/MySQL 执行历史存档。归档功能是旁路写入：数据库不可用或写入失败时，server 会记录日志并继续保留现有执行、WebSocket 广播和 `/api/results` 内存历史逻辑。

适用版本：

- MariaDB 10.5。
- `registerserver/rustserver` 已包含 `QA_EXECUTION_ARCHIVE_MYSQL_ENABLED` 归档开关。

## 部署前确认

1. MariaDB 服务已安装并启动。
2. 有 MariaDB root 或具备建库、建用户、授权权限的账号。
3. register server 能访问 MariaDB 的 host 和 port，默认端口是 `3306`。
4. 测试环境和线上环境建议使用不同数据库名、不同账号或至少不同密码。

## 环境命名建议

测试环境可以直接使用默认示例：

```text
数据库名：qa_register_archive
业务账号：qa_register
```

线上环境建议显式区分：

```text
数据库名：qa_register_archive_prod
业务账号：qa_register_prod
```

下文 SQL 默认使用测试环境示例。如果用于线上，把 SQL 中的数据库名、账号和密码替换为线上值。

## 1. 登录 MariaDB

在数据库所在机器执行：

```powershell
mariadb -u root -p
```

如果本机只有 MySQL 兼容客户端，也可以使用：

```powershell
mysql -u root -p
```

## 2. 创建数据库和账号

同机部署时，执行：

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

如果 register server 和 MariaDB 不在同一台机器，需要把来源 host 换成 register server 的实际 IP：

```sql
CREATE USER IF NOT EXISTS 'qa_register'@'<APP_SERVER_IP>'
  IDENTIFIED BY 'qa_register_password';

GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX
  ON qa_register_archive.*
  TO 'qa_register'@'<APP_SERVER_IP>';

FLUSH PRIVILEGES;
```

线上不要使用示例密码，改成强密码，并同步到 `.env` 连接串。

## 3. 创建迁移记录表

```sql
USE qa_register_archive;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version VARCHAR(64) NOT NULL,
  description VARCHAR(255) NOT NULL,
  applied_at TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
  PRIMARY KEY (version)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
```

## 4. 创建执行记录表

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

## 5. 记录 schema 版本

```sql
USE qa_register_archive;

INSERT INTO schema_migrations (version, description)
VALUES ('20260519_001', 'create execution archive tables')
ON DUPLICATE KEY UPDATE description = VALUES(description);
```

## 6. 配置 register server

编辑 `registerserver/rustserver/.env`：

```env
QA_EXECUTION_ARCHIVE_MYSQL_ENABLED=true
QA_EXECUTION_ARCHIVE_MYSQL_URL=mysql://qa_register:qa_register_password@127.0.0.1:3306/qa_register_archive
QA_EXECUTION_ARCHIVE_MYSQL_MAX_CONNECTIONS=5
QA_EXECUTION_ARCHIVE_QUEUE_SIZE=10000
```

如果 MariaDB 在另一台机器，把连接串里的 `127.0.0.1` 改成数据库地址：

```env
QA_EXECUTION_ARCHIVE_MYSQL_URL=mysql://qa_register:qa_register_password@<DB_HOST>:3306/qa_register_archive
```

不要提交真实 `.env`。仓库只提交 `.env.example`。

## 7. 重启 register server

```powershell
cd registerserver/rustserver
cargo run
```

启动日志中看到以下事件，表示归档已启用：

```json
{"event":"execution_archive_enabled","backend":"mysql"}
```

如果看到以下事件，表示归档被禁用，但主服务仍会继续启动：

```json
{"event":"execution_archive_disabled","reason":"..."}
```

## 8. 执行验证

通过 Web 控制台、HTTP API 或 qamcp 执行一次 QaTest 方法后，在 MariaDB 中查询：

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

正常情况下，同一个 `request_id` 只保留一条业务记录：开始时写入 `running`，结束后 upsert 更新为 `success`、`failed` 或 `cancelled`。

## 9. 关闭归档

需要临时关闭时，只改 `.env` 并重启 server：

```env
QA_EXECUTION_ARCHIVE_MYSQL_ENABLED=false
```

关闭后不会连接 MariaDB，也不会继续写入执行记录。已写入的数据不会被删除。

## 常见问题

### 启动时报 `mysql_connect_failed`

检查：

- `QA_EXECUTION_ARCHIVE_MYSQL_URL` 是否正确。
- MariaDB 是否启动。
- 数据库账号密码是否正确。
- register server 所在机器是否被 MariaDB 授权访问。
- 防火墙是否允许访问 `3306`。

### 执行后没有新记录

检查 server 日志是否有：

- `execution_archive_enabled`
- `execution_archive_enqueue_failed`
- `execution_archive_write_failed`

如果有 `execution_archive_write_failed`，优先检查表是否创建、账号是否有 `INSERT/UPDATE` 权限。

### 线上权限建议

- 业务账号只授予当前库权限，不使用 root 连接。
- 密码放在部署机器 `.env` 或密钥管理系统中，不提交仓库。
- MariaDB 只允许 register server 来源地址访问。
- 定期备份 `execution_records`，并按实际数据量规划归档或清理策略。
