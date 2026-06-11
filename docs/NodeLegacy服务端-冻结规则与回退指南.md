# NodeLegacy 服务端：冻结规则与回退指南

`registerserver/server` 是历史 Node.js 服务端实现，当前已经冻结为 legacy。后续 register server 只以 `registerserver/rustserver` 为主实现，Node server 不再同步新能力、协议字段、状态机事件、Web 控制台预期或 qamcp 行为。

## 当前定位

- Rust server 是唯一主实现。
- Node server 仅保留为历史行为参考和紧急回退入口。
- 正常开发、验证、发布和文档示例都必须使用 Rust server。
- 新功能、bug fix、协议变更、状态机变更和生产行为只修改 Rust server。
- 不再为 Node server 建立新的协议测试、功能测试或发布验收要求。

## 允许使用场景

- 对照历史行为，辅助理解早期协议实现。
- Rust server 临时不可用时，在本地做短时间回退验证。
- 迁移排查时确认某个历史行为是否来自旧 Node 实现。

## 不再支持场景

- 不作为生产或默认开发服务端。
- 不要求兼容 Rust server 后续新增字段或事件。
- 不要求同步 Web 控制台、qamcp、Unity client 的新能力。
- 不接受常规功能开发、协议补齐或状态机修复。

## 回退运行步骤

先确保 Rust server 已停止，因为 Node legacy 默认也监听 `3000` 端口。

```powershell
cd registerserver/server
npm install
npm start
```

如果需要 Web 控制台静态文件，先构建前端：

```powershell
cd registerserver/client
npm install
npm run build
```

回退时 Unity、Web 控制台和 qamcp 仍连接同一个 QA server 地址：

```text
http://localhost:3000
ws://localhost:3000/ws
```

qamcp 如需指定地址：

```powershell
qamcp config set serverUrl http://localhost:3000
```

## 回退风险

- Node legacy 可能缺少 Rust server 后续新增字段、事件和状态收敛逻辑。
- 文档、Web 控制台和 qamcp 的最新行为以 Rust server 为准；Node legacy 上出现差异时，不再反向修改 Rust 行为。
- 如果回退发现 Node 与 Rust 行为不一致，默认记录为 Node legacy 差异；除非只是文档补充，不再更新 Node server 代码。

## 最小检查

只有确实需要临时运行 Node legacy 时，才执行：

```powershell
cd registerserver/server
node --check src/index.js
npm start
```

健康检查：

```powershell
Invoke-RestMethod -Uri "http://localhost:3000/api/health"
```
