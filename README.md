# QA Test Framework

这是一个面向 Unity 应用的 QA 自动化测试框架。Unity 运行时通过 `[QaTest]` 注册可执行测试方法，register server 负责客户端注册、请求调度、状态维护和结果广播，Web 控制台与 MCP 工具分别提供人工操作和 AI Agent 自动调用入口。

## 模块

- `unityclient/My project`: Unity 2022.3 示例工程；`unityclient/My project/Assets/scripts` 是 Unity package 源码目录。
- `registerserver/rustserver`: Rust + Axum + Tokio 注册调度服务，作为 register server 唯一主实现维护。
- `registerserver/server`: Node.js + Express + WebSocket legacy 注册调度服务，已冻结，仅保留历史参考和紧急回退入口。
- `registerserver/client`: Vue 3 + Element Plus Web 控制台。
- `qamcp`: stdio MCP 工具服务，供 AI Agent 调用 QA 能力。
- `docs`: 运行、架构、协议和故障处理文档。

## 仓库边界

当前采用单一 Git 仓库。根目录统一跟踪项目文档、registerserver、qamcp、Unity 示例工程和 Unity package 源码。

根目录 `.gitignore` 只忽略依赖、构建产物、本地配置和 Unity 生成目录，不忽略各模块源码目录。提交前在根目录检查整体状态：

```powershell
git status --short
```

Unity package 源码在 `unityclient/My project/Assets/scripts`。修改 Unity C#、package、Samples 或 `.meta` 文件后，和其他模块一样在根仓库统一提交。

推送当前仓库可直接使用：

```powershell
.\push-all.ps1
```

## 文档入口

- [项目总览-架构流程与后续计划](./docs/项目总览-架构流程与后续计划.md)
- [快速开始-本地跑通 QA 链路](./docs/快速开始-本地跑通QA链路.md)
- [运行指南-启动构建与本地联调](./docs/运行指南-启动构建与本地联调.md)
- [配置参考-端口超时鉴权与执行限制](./docs/配置参考-端口超时鉴权与执行限制.md)
- [RegisterServer-服务端 API 与 WebSocket 协议](./docs/RegisterServer-服务端API与WebSocket协议.md)
- [UnityClient-接入 QaTest 与运行时行为](./docs/UnityClient-接入QaTest与运行时行为.md)
- [QAMCP-Agent 工具使用指南](./docs/QAMCP-Agent工具使用指南.md)
- [WebSocket 状态收敛-断线超时锁与 Late Result](./docs/WebSocket状态收敛-断线超时锁与LateResult.md)
- [NodeLegacy 服务端-冻结规则与回退指南](./docs/NodeLegacy服务端-冻结规则与回退指南.md)
- [MariaDB 执行历史存档-部署与验证手册](./docs/MariaDB执行历史存档-部署与验证手册.md)
- [MariaDB/MySQL 执行历史存档-设计方案](./docs/MariaDBMySQL执行历史存档-设计方案.md)
- [能力边界-已支持与暂不支持](./docs/能力边界-已支持与暂不支持.md)
- [发布版本矩阵-仓库 Commit 与协议版本](./docs/发布版本矩阵-仓库Commit与协议版本.md)
- [发布检查清单-构建验证与冒烟](./docs/发布检查清单-构建验证与冒烟.md)
- [变更记录-项目治理与文档调整](./docs/变更记录-项目治理与文档调整.md)

## 快速启动

启动服务端：

```powershell
cd registerserver/rustserver
cargo run
```

内网部署时如需给 Web 控制台加登录 token，可在启动前设置 `QA_WEB_CONSOLE_TOKEN`；Unity 和 qamcp 调用方式不变。

构建 Web 控制台：

```powershell
cd registerserver/client
npm install
npm run build
```

用 Unity 2022.3 打开 `unityclient/My project` 并进入 Play Mode。Editor 下 QA 功能默认启用，客户端会默认连接：

```text
ws://localhost:3000/ws?role=unity
```

构建后的 Player 默认关闭 QA 功能；测试包需要通过 `--qa-test-enable`、环境变量 `QA_TEST_ENABLED=1`、PlayerPrefs `QaTest.Enabled=1` 或 `QaTestClient.SetGlobalEnabled(true)` 显式开启。

需要让 AI Agent 使用 QA 能力时，配置并启动根目录下的 `qamcp`。详见 [QAMCP-Agent 工具使用指南](./docs/QAMCP-Agent工具使用指南.md)。

Node 版 register server 后续不再更新；相关回退说明见 [NodeLegacy 服务端-冻结规则与回退指南](./docs/NodeLegacy服务端-冻结规则与回退指南.md)。
