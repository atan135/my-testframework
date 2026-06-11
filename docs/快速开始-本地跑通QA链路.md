# 快速开始：本地跑通 QA 链路

本文只覆盖第一次把 QA 链路跑通所需的最小步骤。更完整的架构、配置和协议说明请继续阅读本目录下的专题文档。

## 适用对象

- 想在 Unity Editor 中验证 `[QaTest]` 方法的人。
- 想通过 Web 控制台手动执行 QA 方法的人。
- 想让 AI Agent 通过 qamcp 调用 QA 能力的人。

## 一、启动 Register Server

命令默认以仓库根目录 `c:\project\qatestframework` 为参照。

首次使用建议先构建 Web 控制台：

```powershell
cd registerserver/client
npm install
npm run build
```

启动 Rust register server：

```powershell
cd ../rustserver
cargo run
```

默认服务地址：

- Web 控制台：`http://localhost:3000`
- 健康检查：`http://localhost:3000/api/health`
- Unity WebSocket：`ws://localhost:3000/ws?role=unity`

需要确认服务端是否正常时：

```powershell
curl.exe http://localhost:3000/api/health
```

## 二、启动 Unity 客户端

1. 用 Unity `2022.3.62f1` 打开 `unityclient/My project`。
2. 进入 Play Mode。
3. Unity Console 出现 `[QaTest] Connected.` 表示连接成功。

Editor 下 QA 功能默认启用，不需要手动把组件放进场景。构建后的 Player 默认关闭 QA 功能，测试包需要显式启用：

```powershell
Game.exe --qa-test-enable --qa-server-url ws://localhost:3000/ws
```

如果 register server 不在本机，把 `localhost` 换成服务端内网 IP。

## 三、用 Web 控制台执行一次测试

1. 打开 `http://localhost:3000`。
2. 在测试控制台确认出现 Unity 实例。
3. 选择 `连通性检查` 或其他已注册方法。
4. 点击执行。
5. 确认返回结果为 `pong`，并在执行记录中看到 `success`。

常用页面：

- `/`：测试控制台，执行单个 `[QaTest]` 方法。
- `/sequences`：请求序列，按顺序执行多个方法。
- `/history`：执行记录，查看最近结果。

## 四、编写最小 QaTest 方法

在 Unity 工程中引用命名空间，并给方法添加 `[QaTest]`：

```csharp
using QaTestFramework;

public static class SmokeQaTests
{
    [QaTest("连通性检查")]
    private static string Ping()
    {
        return "pong";
    }
}
```

支持静态方法，也支持场景中已存在的 `MonoBehaviour` 实例方法。参数会从 Web 控制台输入转换为常用类型，例如 `string`、`bool`、数字、枚举和 `Vector2/3/4`。

## 五、让 AI Agent 使用 qamcp

只有需要 AI Agent 调用 QA 能力时才需要配置 qamcp。

构建 Windows exe：

```powershell
cd qamcp
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build-release.ps1
```

保存 QA register server 地址：

```powershell
.\dist\qamcp.exe config set serverUrl http://localhost:3000
```

MCP 客户端配置示例：

```json
{
  "mcpServers": {
    "qamcp": {
      "command": "C:\\project\\qatestframework\\qamcp\\dist\\qamcp.exe",
      "args": ["mcp"]
    }
  }
}
```

常用 qamcp 能力：

- 查看 QA server 是否可用。
- 查看在线 Unity 实例。
- 搜索已注册 `[QaTest]` 方法。
- 执行单个方法或请求序列。
- 查询最近执行结果。

## 六、最常见问题

### Web 控制台没有 Unity 实例

确认 register server 已启动、Unity 已进入 Play Mode，并且 Unity Console 出现 `[QaTest] Connected.`。如果是 Player 包，确认启动时传入了 `--qa-test-enable`，或使用环境变量、PlayerPrefs、运行时 API 启用 QA。

### 方法列表为空

确认目标方法已添加 `[QaTest]`。如果是实例方法，确认对应 `MonoBehaviour` 已存在于当前场景中。

### 执行一直 running

确认 Unity 没有卡住，目标方法、`Task` 或协程最终会返回。服务端默认单步执行超时为 `20000` 毫秒。

### Web 控制台需要登录

说明服务端配置了 `QA_WEB_CONSOLE_TOKEN`。输入对应 token 后继续使用。该 token 不影响 Unity 和 qamcp。

## 七、继续阅读

- [运行指南-启动构建与本地联调](./运行指南-启动构建与本地联调.md)
- [配置参考-端口超时鉴权与执行限制](./配置参考-端口超时鉴权与执行限制.md)
- [UnityClient-接入 QaTest 与运行时行为](./UnityClient-接入QaTest与运行时行为.md)
- [RegisterServer-服务端 API 与 WebSocket 协议](./RegisterServer-服务端API与WebSocket协议.md)
- [QAMCP-Agent 工具使用指南](./QAMCP-Agent工具使用指南.md)
