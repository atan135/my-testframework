# UnityClient：接入 QaTest 与运行时行为

`unityclient` 是 QA Test Framework 的 Unity 运行时客户端。QA 功能启用后，客户端会在 Unity 进入 Play Mode 或 Player 启动时连接 QA register server，扫描当前 AppDomain 中带 `[QaTest]` 的方法并注册到服务端，然后接收 Web 控制台或 MCP 工具下发的执行指令。

当前示例工程位于：

```text
unityclient/My project
```

客户端包源码位于：

```text
unityclient/My project/Assets/scripts
```

## 运行环境

- Unity 2022.3。当前工程版本是 `2022.3.62f1`。
- .NET 运行环境需支持 `System.Net.WebSockets.ClientWebSocket`。
- QA register server 已启动，默认地址为 `ws://localhost:3000/ws?role=unity`。

## 目录结构

```text
Assets/scripts/
  package.json
  README.md
  Runtime/
    QaTestFramework.UnityClient.asmdef
    QaTest/
      QaTestAttribute.cs
      QaTestBootstrap.cs
      QaTestClient.cs
      QaTestClientName.cs
      QaTestCoroutineResult.cs
      QaTestCoroutineReturn.cs
      QaTestDtos.cs
      QaTestLog.cs
      QaTestMethodEntry.cs
      QaParamAttribute.cs
      QaTestRegistry.cs
  Samples~/
    Example/
      QaTestSample.cs
      QaTestStaticSample.cs
      QaTestUtilitySample.cs
      QaTestPanel.cs
      QaTestControl.cs
```

核心代码在 `Runtime/QaTest/`，示例测试方法在 `Samples~/Example/`。

## 接入方式

### 作为本地包接入

把 `unityclient/My project/Assets/scripts` 作为 Unity package 添加到目标 Unity 工程。该目录包含 package manifest：

```json
{
  "name": "com.qatestframework.unityclient",
  "version": "0.1.3",
  "displayName": "QA Test Framework Unity Client",
  "unity": "2022.3"
}
```

如果在当前示例工程内验证，直接用 Unity 打开 `unityclient/My project` 即可。

### 通过 Git URL 接入

在目标工程的 `Packages/manifest.json` 中添加依赖。仓库地址按实际部署替换：

```json
{
  "dependencies": {
    "com.qatestframework.unityclient": "https://your-git-host/your-repo.git?path=/unityclient/My%20project/Assets/scripts"
  }
}
```

如果曾经安装过旧结构包，建议先在 Package Manager 中 remove 后重新 add；必要时清理 `Packages/packages-lock.json` 中对应条目。

## 启动和连接

`QaTestBootstrap` 使用 `RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.AfterSceneLoad)` 处理启动创建。Editor 下仍按 QA 启用状态决定是否自动创建；Player 下默认不创建，只有 PlayerPrefs `QaTest.AutoConnectOnStartup=1` 时才会在启动后调用 `QaTestClient.StartConnect()`。

默认连接地址：

```text
ws://localhost:3000/ws?role=unity
```

连接成功后 Unity 控制台会输出：

```text
[QaTest] Connected.
```

客户端会：

1. 生成或读取持久化的 `clientId`。
2. 扫描 `[QaTest]` 方法。
3. 发送 `register` 消息到服务端。
4. 默认每 10 秒发送一次 heartbeat。
5. 连接失败后默认每 2 秒重连。

如果服务端返回 fatal error 且 `code` 为 `duplicate_client_name`，表示当前 `clientName` 已被其他 `clientId` 注册。客户端会输出中文 error 日志，并在报错中说明重复的名称；随后停止重连，并在 Editor 下退出 Play Mode，在 Player 下调用 `Application.Quit(1)` 结束启动。

## 启用开关

Unity QA 功能默认策略：

- Editor 默认启用，方便本地 Play Mode 调试。
- Player 默认关闭，避免正式包默认暴露 QA WebSocket 能力。

启用状态按以下优先级解析：

1. 运行时 API 覆盖。
2. 命令行参数。
3. 环境变量 `QA_TEST_ENABLED`。
4. PlayerPrefs key `QaTest.Enabled`。
5. Inspector 默认值 `enableInEditor` 或 `enableInPlayer`。

### 运行时 API

外部业务代码可以直接控制全局开关：

```csharp
using QaTestFramework;

QaTestClient.SetGlobalEnabled(true);   // 持久化启用；如果当前没有客户端，会自动创建并连接
QaTestClient.SetGlobalEnabled(false);  // 持久化关闭并断开连接
QaTestClient.ClearGlobalEnabled();     // 清除 PlayerPrefs 和运行时覆盖
```

`SetGlobalEnabled(false, persist: false)` 可以在客户端创建前临时阻止本次进程自动创建；`persist: true` 会写入 PlayerPrefs，影响后续启动。

Player 包推荐由游戏上层主动写入连接配置并启动：

```csharp
QaTestClient.SetIpAndPort("192.168.1.10", 3000);
QaTestClient.SetClientName("Phone-LoginSmoke");
QaTestClient.StartConnect();

// 或一次性保存配置并启动连接。
QaTestClient.StartConnect("192.168.1.10", 3000, "Phone-LoginSmoke");

// 仅 Player 生效；开启后下次游戏启动会自动调用 StartConnect()。
QaTestClient.SetAutoConnectOnStartup(true);
```

`SetIpAndPort`、`SetClientName` 和 `SetAutoConnectOnStartup` 总是写入 PlayerPrefs。`StartConnect` 会先读取 PlayerPrefs 中的 `serverIP`、`serverPort`、`clientName` 和 `clientId`，再创建并启用 `[QaTestClient]`，确保首次注册使用上层保存的名称。

如果已经持有 `QaTestClient` 实例，也可以调用：

```csharp
client.SetClientEnabled(true);
client.SetClientEnabled(false);
```

### 命令行

启动 Editor 或构建后的 Player 时可显式启用或关闭：

```powershell
Unity.exe -projectPath <projectPath> --qa-test-enable
Game.exe --qa-enabled=true
Game.exe --qa-test-disable
```

也支持：

```powershell
--qa-test-enable
--qa-test-disable
--qa-test-enabled=true
--qa-test-enabled=false
```

### 环境变量

```powershell
$env:QA_TEST_ENABLED = "1"
```

可识别值包括 `1/0`、`true/false`、`yes/no`、`on/off`、`enabled/disabled`。

### PlayerPrefs

```csharp
PlayerPrefs.SetInt(QaTestClient.EnabledPlayerPrefsKey, 1);
PlayerPrefs.Save();
```

关闭：

```csharp
PlayerPrefs.SetInt(QaTestClient.EnabledPlayerPrefsKey, 0);
PlayerPrefs.Save();
```

### Inspector

在场景中预置 `QaTestClient` 时，可以通过 Inspector 配置：

- `enableInEditor`: Editor 默认启用状态，默认 `true`。
- `enableInPlayer`: Player 默认启用状态，默认 `false`。

运行时 Inspector 的 Runtime Diagnostics 会显示 `QA Enabled` 和 `Enabled Source`，用于确认当前开关来源。

## 覆盖服务地址

### 命令行参数

启动 Unity 或构建后的 Player 时传入：

```powershell
--qa-server-url ws://localhost:3000/ws
```

也支持等号写法：

```powershell
--qa-server-url=ws://localhost:3000/ws
```

如果 URL 中没有 `role=` 参数，客户端会自动补齐 `role=unity`。

### PlayerPrefs

运行时写入：

```csharp
QaTestClient.SetIpAndPort("localhost", 3000);
QaTestClient.GetIpAndPort(out string ip, out int port);
```

### Inspector

如果需要更细的控制，可以在场景中预置一个带 `QaTestClient` 的对象，并通过 Inspector 配置：

- `enableInEditor`: Editor 默认启用状态。
- `enableInPlayer`: Player 默认启用状态。
- `serverIP`: register server IP 或 host。
- `serverPort`: register server 端口。
- `clientName`: 客户端显示名称。Editor 下在 Inspector 修改后会立即保存到项目根目录 `qatest.config.txt`。
- `reconnectDelaySeconds`: 重连间隔，默认 2 秒。
- `heartbeatSeconds`: 心跳间隔，默认 10 秒。

场景里如果已经存在 `QaTestClient`，自动启动器不会再创建新的对象。

## 客户端名称

默认名称为 `clientId` 的前 8 位：

```text
3f0a9c12
```

可以用 API 修改：

```csharp
using QaTestFramework;

QaTestClientName.Set("WindowsEditor-LoginSmoke", persist: true);
QaTestClient.SetClientName("Phone-LoginSmoke");
```

常用 API：

- `QaTestClientName.Set(name, persist, resendRegister)`: 设置名称。
- `QaTestClientName.Clear(persist, resendRegister)`: 清除自定义名称。
- `QaTestClientName.GetCustom()`: 获取自定义名称。
- `QaTestClientName.GetResolved()`: 获取最终展示名称。
- `QaTestClientName.RefreshRegistration()`: 重新扫描当前 `[QaTest]` 方法并主动刷新注册信息。

Editor 下用户修改后的 `clientId` 和 `clientName` 会保存到 Unity 项目根目录的 `qatest.config.txt`。Player 下外部配置只认 PlayerPrefs：`QaTest.ClientId`、`QaTest.ClientName`、`QaTest.ServerIP`、`QaTest.ServerPort` 和 `QaTest.AutoConnectOnStartup`；Player 不再读取或写入 `Application.persistentDataPath/qatest.config.txt`。

Editor 下如果 `qatest.config.txt` 不存在，客户端会继续使用由 `clientId` 推导出的默认名称，不会自动把默认名写入本地文件。只要用户在 Inspector 或持久化 API 中修改过一次名称，客户端就会创建或更新项目根目录 `qatest.config.txt`；后续任意场景或任意位置启用的 `QaTestClient` 都会读取该文件中的 `clientName`，并同步显示到 Inspector。再次修改 Inspector 会立即更新该本地配置。

Player 下 `clientName` 从 PlayerPrefs `QaTest.ClientName` 读取；没有设置时用 `clientId` 前 8 位作为默认名称。删除 PlayerPrefs 中的 `QaTest.ClientName` 会回到默认名称，删除 `QaTest.ClientId` 会生成新的身份。

首次启动时，如果配置文件不存在或缺少有效 `clientId`，客户端会按平台生成种子并追加随机值，再计算 SHA-256 hash，并截取为 32 位小写 hex：

- Editor: 使用 Unity 项目根目录。
- Player: 使用 `Application.identifier` 和 `Application.platform`，并写入 PlayerPrefs `QaTest.ClientId`。

旧的 64 位 `clientId` 会在读取后规范为前 32 位并回写。

也可以通过内置 QA 方法 `设置客户端名称` 远程修改客户端名称，传空字符串会恢复为默认名称。Editor 下写入 `qatest.config.txt`；Player 下写入 PlayerPrefs。

## 编写 QaTest 方法

引用命名空间后给方法添加 `[QaTest]`：

```csharp
using QaTestFramework;
using UnityEngine;

public sealed class LoginQaTests : MonoBehaviour
{
    [QaTest("点击登录按钮", "模拟点击登录按钮并返回结果。")]
    private static string ClickLoginButton(string objectName)
    {
        return "clicked: " + objectName;
    }
}
```

扫描规则：

- 支持 `public`、`private`、`static`、实例方法。
- 不注册泛型方法。
- 静态方法会直接注册。
- 实例方法必须定义在 `MonoBehaviour` 类型上，并且场景中存在该组件实例。
- 当前公开给 server/Web/qamcp 的 `id` 使用短签名格式，要求方法名和短签名全局唯一。
- 实例方法内部会保留包含声明类型和 Unity 对象 instance id 的 full id 用于缓存和查找，但当前注册消息只上报短签名 `id`。
- 同一个实例 `[QaTest]` 方法如果在场景中解析到多个 `MonoBehaviour` 目标，当前会拒绝注册并报错，而不是用 instance id 暴露多个同名实例方法。

`[QaTest]` 支持以下写法：

```csharp
[QaTest]
[QaTest("显示名称")]
[QaTest("显示名称", "描述文本")]
[QaTest("显示名称", "描述文本", true)]
[QaTest("显示名称", "描述文本", AllowParallelExecution = true)]
```

如果没有提供显示名称，Web 控制台会使用方法名。

`AllowParallelExecution = true` 适合只读查询类方法。Unity 注册时会上报 `allowParallelExecution`，Rust server 对同一 `clientId` 的这类执行不会按本地 busy、运行中请求或其它控制端锁做互斥拦截，并会把并行标记下发给 Unity client。未设置时仍保持单客户端串行执行。

## 方法 ID

注册给 server 的方法 ID 由方法名和参数类型生成：

```text
Method(System.String,System.Boolean)
```

客户端内部还会维护完整方法 ID，用于区分声明类型和实例目标：

```text
Namespace.Type.Method(System.String)@12345
```

通常不需要手写 method id，Web 控制台和 MCP 工具会使用 Unity 注册时上报的 `id`。

## 参数转换

服务端会把参数统一作为字符串数组传给 Unity。客户端按方法签名转换类型。

支持的参数类型：

- `string`
- `bool`: `"true"` 或 `"1"` 为 true，其他值为 false。
- `int`
- `long`
- `float`
- `double`
- `enum`: 忽略大小写解析枚举名。
- `Vector2`: `"x,y"`。
- `Vector3`: `"x,y,z"`。
- `Vector4`: `"x,y,z,w"`。
- 可空类型，例如 `int?`。
- 可选参数，空值时使用默认值。
- 其他可被 `JsonUtility.FromJson` 解析的类型。

当参数为空字符串或缺失时：

- 缺失的必填参数会执行失败，错误形如 `missing required argument: <name>`。
- 缺失的可选参数使用声明中的默认值；没有声明默认值时，`string` 为空字符串、值类型为默认值、引用类型为 `null`。
- 空字符串对必填参数会执行失败，避免错误输入被静默转成 `0`、`false` 或默认对象。
- 如果业务上确实需要传空字符串，请传入显式占位 JSON：`{"$empty":true}`；该占位只允许用于 `string` 参数。

## 返回值

测试方法支持以下返回方式：

- 普通返回值：用 invariant culture 转成字符串。
- `Task`: 等待完成后返回 `Task completed`。
- `Task<T>`: 等待完成后返回 `T` 的字符串值。
- `IEnumerator`: 作为 Unity 协程运行，完成后返回协程结果。
- `QaTestCoroutineResult`: 适合通过回调或闭包在协程结束后提供最终结果。

普通返回示例：

```csharp
[QaTest("连通性检查")]
private static string Ping()
{
    return "pong";
}
```

协程通过 `QaTestCoroutineReturn` 返回结果：

```csharp
[QaTest("等待后返回")]
private static System.Collections.IEnumerator WaitAndReturn(float seconds = 1f)
{
    yield return new WaitForSeconds(seconds);
    yield return QaTestCoroutineReturn.From("done");
}
```

通过 `QaTestCoroutineResult` 返回结果：

```csharp
[QaTest("等待面板显隐状态")]
private static QaTestCoroutineResult WaitPanelActive(string objectName)
{
    string result = string.Empty;
    return new QaTestCoroutineResult(
        WaitRoutine(objectName, value => result = value),
        () => result);
}
```

如果返回的是 `UnityEngine.Object`，结果字符串会使用对象的 `name`。

如果返回字符串 `failed:<reason>`、`false`，或返回包含 `ok=false` / 失败态 `status` 的结构化 JSON，客户端会把业务失败映射为 `success=false` 并填充 `error`。server 侧也会对迟到或兼容路径中的同类结果做兜底判定。

## Package 和 `.meta` 策略

Unity 客户端以 package 方式组织。当前 `unityclient/My project/Assets/scripts` 目录会跟踪 `.meta` 文件；新增脚本、asmdef、Samples 或资源时，必须同步提交对应 `.meta` 文件。

不要把 Unity package 内的 `.meta` 当作生成文件删除或忽略。`.meta` 中的 GUID 会影响 Unity package 在接入工程中的引用稳定性。

## 执行链路

1. QA register server 启动。
2. Unity 进入 Play Mode 或 Player 启动，QA 功能启用时 `QaTestBootstrap` 创建 `[QaTestClient]`。
3. 客户端连接 `ws://localhost:3000/ws?role=unity`；如果 QA 功能关闭，则不会自动创建或连接。
4. 客户端扫描 `[QaTest]` 方法并发送 `register`。
5. Web 控制台或 MCP 工具发送 `execute`。
6. Unity 主线程执行目标方法。
7. Unity 回传 `qa_result`。
8. 服务端广播结果并写入执行历史。

## 与服务端的消息格式

注册消息：

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
  "methods": [
    {
      "id": "Method()",
      "name": "显示名称",
      "declaringType": "Namespace.Type",
      "description": "方法说明",
      "returnType": "String",
      "isStatic": true,
      "allowParallelExecution": true
    }
  ],
  "busy": false,
  "currentRequestId": "",
  "currentMethodName": ""
}
```

执行命令：

```json
{
  "type": "execute",
  "requestId": "uuid",
  "methodId": "Method()",
  "methodName": "显示名称",
  "allowParallelExecution": true,
  "arguments": []
}
```

执行结果：

```json
{
  "type": "qa_result",
  "requestId": "uuid",
  "clientId": "client-id",
  "methodId": "Method()",
  "methodName": "显示名称",
  "success": true,
  "result": "pong",
  "error": "",
  "durationMs": 5,
  "busy": false,
  "currentRequestId": "",
  "currentMethodName": ""
}
```

心跳消息：

```json
{
  "type": "heartbeat",
  "clientId": "client-id",
  "busy": false,
  "currentRequestId": "",
  "currentMethodName": ""
}
```

## 示例脚本

`Samples~/Example/` 提供以下示例：

- `QaTestStaticSample`: static class 静态方法注册示例。
- `QaTestUtilitySample`: 普通 class 静态方法注册示例。
- `QaTestSample`: MonoBehaviour 实例方法、静态方法、输出日志、等待后返回。
- `QaTestPanel`: 面板存在性、显隐设置、显隐状态查询、等待显隐状态。
- `QaTestControl`: 按钮点击、控件可交互状态设置、可交互状态检查、等待可交互状态。

这些示例以 mock 行为为主，适合验证 Unity、register server、Web 控制台和 MCP 工具的完整链路。

## 日志

框架连接日志使用 Unity `Debug.Log` 或 `Debug.LogWarning`，前缀为 `[QaTest]`。

示例方法使用 `QaTestLog` 输出触发和结果日志：

```csharp
QaTestLog.Enabled = false;
```

设置为 `false` 可关闭示例日志输出。

## 校验流程

1. 启动 register server：

```powershell
cd registerserver/rustserver
cargo run
```

2. 用 Unity 2022.3 打开 `unityclient/My project` 并进入 Play Mode。Editor 默认启用 QA 功能，Player 测试包需要显式传入 `--qa-test-enable` 或调用 `QaTestClient.SetGlobalEnabled(true)`。
3. 打开 `http://localhost:3000`。
4. 在“测试控制台”确认出现 Unity 实例。
5. 执行 `连通性检查`，确认返回 `pong`。
6. 在“执行记录”确认状态为 `success`。

## 常见问题

### Unity 没有出现在 Web 控制台

检查 QA 功能是否启用、register server 是否启动、端口是否为 `3000`、Unity 控制台是否有连接失败日志。如果服务端在其他机器，使用 `--qa-server-url` 或 PlayerPrefs 覆盖地址。Player 中默认关闭 QA 功能，需要显式传入 `--qa-test-enable`、设置 `QA_TEST_ENABLED=1`、写入 `QaTest.Enabled=1` 或调用 `QaTestClient.SetGlobalEnabled(true)`。

### 进入 Play Mode 后出现重复客户端

`QaTestClient` 会在 `Awake` 中检查场景内实例数量，超过一个会销毁后创建的对象。建议目标工程只保留一个手动配置的 `QaTestClient`，或者完全依赖自动创建。

### 方法没有注册

检查方法是否添加 `[QaTest]`，方法是否为泛型，实例方法所在的 `MonoBehaviour` 是否已经存在于场景中。客户端连接和 `QaTestClientName.RefreshRegistration()` 时会刷新 registry；执行时优先使用已注册缓存，只有缓存未命中或实例目标失效时才刷新一次并重试。实例方法仍依赖当前场景对象。

### 参数转换失败

确认 Web 控制台输入的字符串符合目标类型格式。`Vector3` 使用英文逗号分隔，例如 `1,2,3`；枚举传枚举名；复杂对象传 `JsonUtility.FromJson` 可解析的 JSON。

### 执行失败但 Web 控制台只看到错误字符串

客户端会捕获异常并回传 `异常类型: 异常消息`。更完整的堆栈需要查看 Unity Console。

### 协程一直不返回

确认协程最终会结束。`QaTestCoroutineReturn.From(value)` 只记录返回值，不会自动结束协程；它后面的逻辑仍会继续执行，直到枚举器结束。
