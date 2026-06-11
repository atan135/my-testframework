# Agent Entry Guide

本文件是 Codex、Claude 或其他 AI Agent 接手本仓库时的入口说明。它只保留项目心智模型、架构边界、基础约定和文档导航；具体接口、协议字段、部署步骤、命令参数和故障细节，以 `docs/` 下专题文档为准。

如果某项能力细节发生变化，优先更新对应专题文档。只有整体架构、模块边界、基础设定或文档导航变化时，才需要同步更新本文件。

## 项目定位

QA Test Framework 是一套面向 Unity 应用的运行时自动化测试调用框架。Unity 端用 `[QaTest]` 暴露可执行能力，register server 负责注册、调度、状态维护和结果广播，Web 控制台服务人工操作，qamcp 服务 AI Agent 自动调用。

核心目标是把 Unity 运行时测试能力标准化、服务化，并让人工 QA、客户端开发和 AI Agent 通过同一条链路发现实例、执行方法、编排请求序列和查看结果。

## 架构边界

```text
qatestframework/
  unityclient/My project/       Unity 2022.3 示例工程和 Unity package
  registerserver/rustserver/    Rust + Axum + Tokio 注册调度主服务
  registerserver/client/        Vue 3 + Element Plus Web 控制台
  registerserver/server/        Node.js legacy 服务端，仅历史参考和紧急回退
  qamcp/                        stdio MCP 工具服务
  docs/                         项目说明、运行、配置、协议和排障文档
```

整体数据流：

```text
AI Agent
  |
  | stdio MCP tools
  v
qamcp
  |
  | HTTP + WebSocket
  v
registerserver/rustserver <------ registerserver/client
  ^
  | WebSocket
  |
unityclient / QaTestClient
```

关键边界：

- `registerserver/rustserver` 是唯一主服务端实现。新增服务端能力、协议字段、状态事件和常规 bug fix 默认改这里。
- `registerserver/server` 是冻结的 Node legacy fallback。不要把它当作双主实现，也不要同步 Rust server 的新能力。
- Web 控制台和 qamcp 在 server 看来都属于 `role=web` 控制端；Unity 属于 `role=unity`。
- Unity 端只负责注册 `[QaTest]` 方法、接收执行指令、在 Unity 主线程执行并回传结果。
- qamcp 不直接连接 Unity；它复用 register server 的 HTTP/WebSocket 能力。
- 执行历史的最近视图由 server 内存维护；长期追溯可使用日志或可选 MariaDB/MySQL 旁路存档。

## 基础设定

- 仓库默认工作目录：`c:\project\my-testframework`。
- 默认服务端地址：`http://localhost:3000`。
- 默认 Unity WebSocket：`ws://localhost:3000/ws?role=unity`。
- Web 控制台生产地址跟随 server；开发地址默认 `http://localhost:5173`。
- Rust server 默认监听 `0.0.0.0`，但 `QA_ACCESS_SCOPE=private` 会限制为本机和内网来源。
- `QA_WEB_CONSOLE_TOKEN` 只保护浏览器控制台登录；当前不影响 Unity 和 qamcp。
- Unity Editor 默认启用 QA 客户端；Player 默认关闭，需要显式启用。
- `.env` 不提交到 Git。配置项以 `registerserver/rustserver/.env.example` 和 `docs/配置参考-端口超时鉴权与执行限制.md` 为准。

## 文档导航

按任务选择对应文档，不要在本文件里补写细节：

- 首次跑通链路：`docs/快速开始-本地跑通QA链路.md`
- 项目总览、整体架构、核心流程和后续 Todo：`docs/项目总览-架构流程与后续计划.md`
- 启动、构建和常用命令：`docs/运行指南-启动构建与本地联调.md`
- 端口、超时、token、heartbeat、执行限制：`docs/配置参考-端口超时鉴权与执行限制.md`
- Register server、HTTP API、WebSocket 协议：`docs/RegisterServer-服务端API与WebSocket协议.md`
- Unity package、`QaTestClient`、`[QaTest]` 写法：`docs/UnityClient-接入QaTest与运行时行为.md`
- qamcp 工具、MCP 客户端配置和 Agent 调用方式：`docs/QAMCP-Agent工具使用指南.md`
- WebSocket 断线、超时、锁、late result：`docs/WebSocket状态收敛-断线超时锁与LateResult.md`
- 当前支持和暂不支持能力：`docs/能力边界-已支持与暂不支持.md`
- Node legacy 定位和紧急回退：`docs/NodeLegacy服务端-冻结规则与回退指南.md`
- MariaDB/MySQL 执行历史存档：`docs/MariaDB执行历史存档-部署与验证手册.md`、`docs/MariaDBMySQL执行历史存档-设计方案.md`
- 发布组合和发布前检查：`docs/发布版本矩阵-仓库Commit与协议版本.md`、`docs/发布检查清单-构建验证与冒烟.md`
- 历史归档：`docs/archive/` 仅用于用户明确要求追溯历史背景时读取；默认不要把归档内容作为当前架构、接口或任务依据。

## Agent 工作原则

- 开始修改前先判断改动属于哪个模块，再阅读对应专题文档和相邻源码。
- 协议或状态字段变化必须同步相关实现端：Rust server、Web 控制台、Unity client、qamcp，以及对应专题文档。
- 不要把实现细节重复写进 `agent.md`；这里应只维护入口级约定和文档索引。
- 不要编辑或提交生成目录，例如 `node_modules/`、`dist/`、Unity `Library/`、`Temp/`、`Logs/`、`UserSettings/`。
- Unity package 目录当前跟踪 `.meta` 文件。新增 Unity 脚本或资源时必须同步对应 `.meta`。
- Unity `JsonUtility` 只可靠序列化字段，协议 DTO 保持 `public field` 风格。
- qamcp 以 stdio MCP 协议通信，MCP 模式不要向 stdout 写普通日志；必要日志写 stderr。
- Web 控制台共享状态集中在 `registerserver/client/src/composables/useQaStore.js`，不要在单个页面重复建立 WebSocket 长连接。
- 安全边界按内网可信部署设计。若要面向非可信网络开放，需要先补齐全局 API/WS 鉴权、白名单、反向代理或权限模型。

## 仓库和提交边界

当前是单一 Git 仓库结构。根目录统一跟踪项目文档、registerserver、qamcp、Unity 示例工程和 Unity package 源码。

提交或检查改动前在根目录查看：

```powershell
git status --short
```

根目录 `.gitignore` 只忽略依赖、构建产物、本地配置和 Unity 生成目录，不忽略 `qamcp/`、`registerserver/` 或 `unityclient/` 源码目录。

提交拆分按逻辑边界处理。协议字段变化如果必须同时影响多端，可以放在同一提交；纯文档、server、client、Unity、qamcp 改动可按审查需要拆开。

## 验证入口

按改动范围选择验证，详细命令见 `docs/运行指南-启动构建与本地联调.md` 和各模块文档：

- Rust server：`cargo fmt`、`cargo check`、`cargo test`
- Web 控制台：`npm run build`
- qamcp：`cargo fmt --check`、`cargo check`、`cargo test`
- Unity C#：优先用 Unity Editor 打开 `unityclient/My project` 做真实编译确认
- Node legacy：只有紧急回退相关变更才检查 `node --check src/index.js`

文档类改动至少检查链接目标是否存在，并确认没有把具体协议字段或部署步骤重新沉淀到本入口文件。
