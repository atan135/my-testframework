# 发布版本矩阵：仓库 Commit 与协议版本

本文档记录 QA Test Framework 发布版本、仓库 commit 和协议版本。当前项目采用单一 Git 仓库，所有模块源码和文档在根仓库统一维护。

## 仓库职责

| 仓库 | 职责 | 维护规则 |
| --- | --- | --- |
| `registerserver/rustserver` | QA register server 唯一主实现，负责 HTTP/WebSocket 协议、状态机、锁控制、执行调度和事件广播。 | 新服务端能力、协议字段和状态机修复只落在 Rust。 |
| `registerserver/server` | Node legacy 服务端。 | 已冻结，不再同步新能力、协议字段、状态机事件或常规修复，仅保留历史参考和紧急回退入口。 |
| `registerserver/client` | Web 控制台。 | 以 Rust server 行为为准。 |
| `qamcp` | MCP stdio 工具和 exe 打包。 | 以 Rust server 协议为准，发布时构建 `dist/qamcp.exe`。 |
| `unityclient/My project/Assets/scripts` | Unity package 客户端。 | 作为单仓中的 package 源码目录维护，按 package 方式部署。 |
| 根目录 | 整体文档、版本矩阵、发布检查清单、脚本和各模块源码。 | 单一 Git 仓库统一提交和发布。 |

## 当前可用组合

| 版本 | 日期 | 仓库 commit | 协议版本 | 说明 |
| --- | --- | --- | --- | --- |
| `0.1.0-dev` | 2026-05-19 | `待记录` | `qa-ws-v1` | 当前开发版本；Rust server 为唯一主实现，Node server 已冻结为 legacy，qamcp 已支持 exe 构建和事件会话，Unity package manifest 版本为 `0.1.3`，Unity 客户端已支持 QA 开关、本地 busy 状态、基础结构化业务失败判定和只读查询类并行执行。 |

发布前必须把 `待记录` 替换为实际仓库 commit。建议使用短 commit hash，并在发布说明中记录对应 tag 或分支。

## 协议变更记录格式

每次 HTTP/WebSocket 消息结构、字段语义或状态机事件变化，都必须追加一条记录：

```markdown
### <日期> <变更标题>

- 协议版本：qa-ws-v1
- 影响 registerserver/rustserver：是/否
- 影响 registerserver/server legacy：是/否/不维护
- 影响 qamcp：是/否
- 影响 Unity client：是/否
- 影响 Web 控制台：是/否
- 是否兼容旧版本：是/否
- JSON 示例：<链接或内联示例>
- 验证记录：<命令或手工验证结果>
```

## 已确认规则

- 采用单一 Git 仓库，不再按 `qamcp`、`registerserver`、`unityclient` 拆分子仓库。
- Unity 端仍按 package 方式部署，但 package 源码作为单仓目录维护。
- Rust server 是 register server 唯一主实现。
- Node server 是冻结的 legacy 实现，不再作为新能力、协议同步或常规修复目标。
- 跨端协议变更必须同步更新受影响模块和文档。
- 根目录统一接管项目源码、文档和发布治理信息。
