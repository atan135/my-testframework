# 示例：兑换活动完整链路 ScenarioIntent

评估日期：2026-05-07

本文档是“人工只提供目标模块或界面，AI 生成测试流程”的样例。它不是最终 `command.json`，而是生成 `command.json` 之前必须确认的场景意图层。

当前仓库的 register server、Unity client 和 qamcp 不直接执行 `ScenarioIntent`。本文属于上层用例生成/WholeClient 接入方案样例，用于说明未来从自然语言目标到可执行命令的设计边界。

## 人工输入

人工只需要提供业务目标，不需要手写详细 UI 步骤：

```text
测试兑换活动完整链路：玩家可以进入兑换活动，完成一次兑换，奖励到账，材料扣除，兑换次数刷新。
```

如果人工知道风险点，可以额外补充：

```text
重点关注：活动页签是否能正确进入，兑换成功后背包道具数量是否增加，兑换次数是否减少。
```

## AI 先生成覆盖矩阵

AI 不应直接生成 `command.json`，应先生成待确认覆盖矩阵：

| 类型 | 覆盖点 | 是否需要人工确认 |
| --- | --- | --- |
| 冒烟 | 活动主界面可以打开 | 否 |
| 冒烟 | 兑换活动页签可以进入 | 是，需确认活动名称或活动 ID |
| 主链路 | GM 准备足够兑换材料 | 是，需确认材料 ID 和数量 |
| 主链路 | 执行一次兑换 | 是，需确认兑换目标奖励 |
| 主链路 | 兑换成功提示出现 | 否 |
| 主链路 | 材料数量减少 | 是，需确认读取来源 |
| 主链路 | 奖励数量增加 | 是，需确认奖励进背包还是邮件 |
| 主链路 | 剩余兑换次数刷新 | 是，需确认以 UI 还是活动数据为准 |
| 异常链路 | 材料不足时不能兑换 | 可选 |
| 异常链路 | 活动关闭时入口不可用 | 可选 |

## ScenarioIntent 样例

```json
{
  "version": 1,
  "scenarioId": "activity_exchange_full_flow",
  "scenarioName": "兑换活动完整链路",
  "module": "兑换活动",
  "goal": "验证玩家可以完成一次活动兑换，并确认材料扣除、奖励到账、兑换次数刷新",
  "humanInput": "测试兑换活动完整链路：玩家可以进入兑换活动，完成一次兑换，奖励到账，材料扣除，兑换次数刷新。",
  "status": "pending_confirmation",
  "preconditions": [
    {
      "id": "logged_in",
      "description": "账号已登录并进入主界面",
      "source": "environment",
      "required": true
    },
    {
      "id": "activity_open",
      "description": "目标兑换活动已开启",
      "source": "gm_or_server_config",
      "required": true
    },
    {
      "id": "material_enough",
      "description": "账号拥有足够兑换材料",
      "source": "gm",
      "required": true
    },
    {
      "id": "bag_available",
      "description": "背包可接收目标奖励",
      "source": "business_state",
      "required": true
    }
  ],
  "ambiguities": [
    {
      "id": "target_activity",
      "question": "兑换活动通过活动 ID、活动标题，还是页签序号定位？",
      "impact": "影响进入 ActivityExchangePanel 的方式和断言稳定性",
      "mustResolveBeforeCommand": true
    },
    {
      "id": "target_exchange_item",
      "question": "兑换哪一个奖励？按奖励 ID、奖励名称，还是第一个可兑换项？",
      "impact": "影响列表匹配、点击目标和奖励到账断言",
      "mustResolveBeforeCommand": true
    },
    {
      "id": "reward_delivery",
      "question": "奖励是否一定进入背包？是否可能进入邮件或临时仓库？",
      "impact": "影响兑换后验证界面和数据 API",
      "mustResolveBeforeCommand": true
    },
    {
      "id": "count_source",
      "question": "兑换次数刷新以 UI 文本为准，还是以活动 model 数据为准？",
      "impact": "影响核心断言来源",
      "mustResolveBeforeCommand": true
    }
  ],
  "routes": [
    {
      "id": "route_to_exchange",
      "from": "MainHud",
      "to": "ActivityExchangePanel",
      "path": [
        "MainHud -> ActivityMainPanel",
        "ActivityMainPanel -> ActivityExchangePanel"
      ]
    },
    {
      "id": "route_to_bag",
      "from": "ActivityExchangePanel",
      "to": "BagPanel",
      "path": [
        "ActivityExchangePanel -> MainHud",
        "MainHud -> BagPanel"
      ]
    }
  ],
  "businessSnapshots": [
    {
      "id": "before_exchange",
      "timing": "before do_exchange",
      "values": [
        {
          "name": "materialCount",
          "source": "QaCommander_bag.GetItemCount(materialItemId)"
        },
        {
          "name": "rewardCount",
          "source": "QaCommander_bag.GetItemCount(rewardItemId)"
        },
        {
          "name": "remainingExchangeCount",
          "source": "QaCommander_act.GetExchangeRemainCount(activityId, exchangeItemId)"
        }
      ]
    },
    {
      "id": "after_exchange",
      "timing": "after exchange result ready",
      "values": [
        {
          "name": "materialCount",
          "source": "QaCommander_bag.GetItemCount(materialItemId)"
        },
        {
          "name": "rewardCount",
          "source": "QaCommander_bag.GetItemCount(rewardItemId)"
        },
        {
          "name": "remainingExchangeCount",
          "source": "QaCommander_act.GetExchangeRemainCount(activityId, exchangeItemId)"
        }
      ]
    }
  ],
  "stages": [
    {
      "id": "prepare_account",
      "intent": "准备活动和兑换材料",
      "expectedState": "MainHud",
      "actions": [
        "EnsureActivityOpen(activityId)",
        "GmAddItem(materialItemId, requiredMaterialCount)"
      ],
      "assertions": [
        "账号在主界面",
        "材料数量大于等于本次兑换消耗"
      ],
      "onFailure": "PRECONDITION_FAILED"
    },
    {
      "id": "open_activity_main",
      "intent": "打开活动主界面",
      "expectedState": "ActivityMainPanel",
      "actions": [
        "OpenPanelByCapability(ActivityMainPanel)",
        "WaitPanelReady(ActivityMainPanel)"
      ],
      "assertions": [
        "ActivityMainPanel 可见",
        "活动列表或页签加载完成"
      ],
      "onFailure": "ROUTE_FAILED"
    },
    {
      "id": "enter_exchange_activity",
      "intent": "进入目标兑换活动",
      "expectedState": "ActivityExchangePanel",
      "actions": [
        "SelectActivityByIdOrTitle(activityId, activityTitle)",
        "WaitUiVisible(ActivityExchangePanel)"
      ],
      "assertions": [
        "目标兑换活动内容可见",
        "兑换列表加载完成",
        "目标兑换项可见且可兑换"
      ],
      "onFailure": "PANEL_NOT_READY"
    },
    {
      "id": "snapshot_before",
      "intent": "记录兑换前业务状态",
      "expectedState": "ActivityExchangePanel",
      "actions": [
        "Read materialCount",
        "Read rewardCount",
        "Read remainingExchangeCount"
      ],
      "assertions": [
        "读取结果有效",
        "remainingExchangeCount 大于 0"
      ],
      "onFailure": "PRECONDITION_FAILED"
    },
    {
      "id": "do_exchange",
      "intent": "执行一次兑换",
      "expectedState": "ActivityExchangePanel",
      "actions": [
        "ClickExchangeItem(exchangeItemId)",
        "HandleBusinessConfirmPanel(exchange_confirm)",
        "WaitExchangeResult(activityId, exchangeItemId)"
      ],
      "assertions": [
        "兑换请求完成",
        "未出现材料不足或次数不足错误"
      ],
      "onFailure": "ASSERTION_FAILED"
    },
    {
      "id": "verify_activity_refresh",
      "intent": "验证活动界面状态刷新",
      "expectedState": "ActivityExchangePanel",
      "actions": [
        "WaitDataChanged(remainingExchangeCount)",
        "Read remainingExchangeCount"
      ],
      "assertions": [
        "after.remainingExchangeCount = before.remainingExchangeCount - 1"
      ],
      "onFailure": "DATA_NOT_CHANGED"
    },
    {
      "id": "open_bag",
      "intent": "打开背包验证奖励",
      "expectedState": "BagPanel",
      "actions": [
        "Navigate(ActivityExchangePanel, BagPanel)",
        "WaitPanelReady(BagPanel)"
      ],
      "assertions": [
        "BagPanel 可见",
        "背包道具列表加载完成"
      ],
      "onFailure": "ROUTE_FAILED"
    },
    {
      "id": "verify_reward",
      "intent": "验证材料扣除和奖励到账",
      "expectedState": "BagPanel",
      "actions": [
        "Read materialCount",
        "Read rewardCount"
      ],
      "assertions": [
        "after.materialCount = before.materialCount - exchangeCost",
        "after.rewardCount = before.rewardCount + rewardAmount"
      ],
      "onFailure": "ASSERTION_FAILED"
    }
  ],
  "crossScreenAssertions": [
    {
      "id": "material_decreased",
      "description": "背包中材料数量按兑换消耗减少",
      "source": "BusinessSnapshot(before_exchange, after_exchange)"
    },
    {
      "id": "reward_increased",
      "description": "背包中奖励数量按兑换配置增加",
      "source": "BusinessSnapshot(before_exchange, after_exchange)"
    },
    {
      "id": "exchange_count_refreshed",
      "description": "活动剩余兑换次数减少 1",
      "source": "ActivityExchange data or UI confirmed source"
    }
  ],
  "recoveryPolicy": [
    {
      "scope": "safe_cover",
      "allowed": [
        "关闭普通提示弹窗",
        "关闭非业务遮挡",
        "返回主界面后重新打开活动"
      ]
    },
    {
      "scope": "business_confirm",
      "allowed": [
        "只有 stage 明确要求 HandleBusinessConfirmPanel(exchange_confirm) 时才允许点击确认"
      ]
    },
    {
      "scope": "dangerous_actions",
      "forbidden": [
        "自动点击购买确认",
        "自动点击删除确认",
        "自动点击消耗钻石确认",
        "在未知弹窗上点击确认"
      ]
    }
  ]
}
```

## 人工确认清单

生成最终 `command.json` 前，人工只需要确认这些点：

1. 目标兑换活动的定位方式：活动 ID、活动标题、页签名或入口路径。
2. 目标兑换项：奖励 ID、奖励名称，或稳定的列表匹配条件。
3. 兑换材料 ID、消耗数量、奖励 ID、奖励数量。
4. 奖励到账位置：背包、邮件、临时仓库，或多种可能。
5. 兑换次数的可信来源：UI 文本、活动 model、服务端返回或配置。
6. 是否需要覆盖材料不足、次数不足、活动未开启等异常链路。

## command 生成规则

确认后，AI 才能把 `ScenarioIntent` 展开为 `command.json`。展开规则：

- 每条 command 必须带 `intentId` 或通过 evidence 映射到 stage。
- 打开界面使用 `PanelCapability`，不要临时猜入口。
- 多界面跳转使用 `TransitionGraph`，不要生成不可恢复的一长串固定点击。
- 界面等待使用 `WaitPanelReady`、`WaitUiVisible`、`WaitNodeVisible`、`WaitDataChanged`，不要用固定 sleep 作为核心判断。
- 业务确认必须显式写成 `HandleBusinessConfirmPanel(exchange_confirm)`，不能由遮挡恢复逻辑自动点击。
- 业务状态验证优先使用 QaTest API 或 model 数据，UI 文本只作为展示层断言。

## 失败归因样例

执行失败时，不应只返回“节点找不到”，而应归因到场景阶段：

| 阶段 | 失败码 | 说明 | 修复方向 |
| --- | --- | --- | --- |
| `open_activity_main` | `ROUTE_FAILED` | 活动入口找不到或被遮挡 | 修复 PanelCapability 或 TransitionGraph |
| `enter_exchange_activity` | `PANEL_NOT_READY` | 活动主界面打开了，但目标兑换内容没进入 | 修复页签定位或活动开启前置条件 |
| `snapshot_before` | `PRECONDITION_FAILED` | 材料不足、次数为 0 或数据读取失败 | 补 GM 或修复业务查询 API |
| `do_exchange` | `ASSERTION_FAILED` | 点击兑换后业务返回失败 | 判断是功能问题还是前置条件错误 |
| `verify_activity_refresh` | `DATA_NOT_CHANGED` | 兑换次数没有刷新 | 判断刷新延迟、UI 未更新或业务逻辑异常 |
| `verify_reward` | `ASSERTION_FAILED` | 奖励未按预期到账 | 确认奖励发放位置或报告功能问题 |

## 与单界面测试的区别

单界面测试关注：

- Panel 是否打开。
- 节点是否存在。
- 点击后是否出现弹窗或 toast。

兑换活动完整链路关注：

- 兑换前后业务数据是否变化。
- 多个界面之间的状态是否一致。
- UI 展示、背包数据、活动次数是否共同证明业务成功。
- 失败时能知道是意图、路由、节点、前置条件还是功能断言出了问题。

因此，这类用例必须先有 `ScenarioIntent`，再生成 `command.json`。
