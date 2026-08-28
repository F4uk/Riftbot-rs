# Nautilus Multi-Venue Perp Arbitrage
## 完整项目任务书 / Architecture · Scope · Coding Standard · Delivery Gates

**文档状态：V2.0 架构冻结稿**  
**文档用途：Codex 主执行规范 + GPT 大阶段审核依据 + 项目长期边界契约**  
**开发模式：Codex 自主推进，小阶段自检修复；大阶段 GPT Gate 审核；用户只参与重大方向、密钥与实盘授权。**

---

# 0. 项目一句话定义

基于 **NautilusTrader** 构建一个 **Rust-first、事件驱动、Delta-Neutral、可回放、可审计、可逐步扩展到多交易所/多腿的 Perp 跨所套利系统**。

系统不预测币价方向，核心目标是：

> 识别不同 Perp Venue 之间的真实可成交价差 → 判断相对公平价差的偏离 → 通过目标库存网格建立 Delta-Neutral 仓位 → 等价差收敛 → 安全减仓/平仓 → 获得净价差收益。

---

# 1. 总体职责划分

## 1.1 NautilusTrader：交易系统基座

Nautilus 负责：

- Event Engine
- Clock
- Cache
- Portfolio
- Order / Fill / Position lifecycle
- LiveNode
- Venue adapters
- 基础 execution/reconciliation 能力
- 回测/实盘事件模型基础

**原则：Nautilus 是 dependency，不 fork、不魔改。**

---

## 1.2 QuantGuy-inspired：Measurement Layer

只吸收其公开方案中对本项目有价值的“市场测量”思想：

- executable bid/ask
- 真实盘口深度
- spread / premium
- fee-aware edge
- midline
- 数据采集
- staleness
- 历史价差统计

QuantGuy 层只回答：

> 当前跨所价格偏离是否真实？偏离多少？扣基础成本后还有多少可交易 edge？

QuantGuy **不负责最终仓位、不直接下单**。

---

## 1.3 CJ-inspired：唯一仓位策略核心

CJ 思路被抽象为：

> spread 偏离扩大 → TargetInventory 增加  
> spread 偏离收缩 → TargetInventory 降低

CJ 层只回答：

> 当前应该持有多少目标套利库存？

系统中不能存在第二套独立仓位策略与其竞争。

---

## 1.4 本项目自研

本项目真正需要形成工程优势的部分：

- RiskManager
- ExecutionBasketCoordinator
- ResidualDeltaHandler
- Reconciliation / Startup Recovery
- Recorder / Replay
- PnL Attribution
- RegimeFilter
- 未来 RouteOptimizer / GlobalInventory / CapitalAllocator

---

# 2. 上游仓库与参考源：必须明确

Codex 不允许自行搜索一个“看起来差不多”的仓库替代本节来源。

## 2.1 NautilusTrader 官方上游

**Repository**

```text
https://github.com/nautechsystems/nautilus_trader
```

**角色**

```text
Production framework dependency
```

**当前参考快照（2026-08-28）**

```text
e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c
```

该 SHA 只作为本任务书生成时的“上游观察快照”。

### Nautilus dependency pin 规则

因为官方 `develop` 更新频繁：

- 禁止长期使用 floating `develop` 作为不可重复依赖。
- P0 必须根据实际 Rust/API/Adapter 兼容性选择：
  - 官方稳定 release/tag，或
  - 经验证的明确 commit SHA。
- 选定后写入 `UPSTREAM_SOURCES.md` 和 lockfile。
- 后续不得自动跟随 upstream 更新。

---

## 2.2 yourQuantGuy 主参考源

**Repository**

```text
https://github.com/your-quantguy/entropy-arb
```

**项目角色**

```text
Measurement reference only
```

**冻结参考 SHA**

```text
aa0391471f6bf72f78c45801fb8117b7bf7e8c89
```

**允许吸收**

- executable spread
- real order book pricing
- fee-aware edge
- midline
- recorder
- statistical analysis
- data freshness ideas

**禁止照搬**

- QuantGuy 自己的最终开仓/平仓策略
- inventory ladder 作为本项目仓位大脑
- Python bot 的完整架构
- venue-specific execution state machine

其仓库当前为 MIT License，但本项目仍优先“重新实现思想”，而不是机械翻译。

---

## 2.3 CJ 主参考源

**Repository**

```text
https://github.com/cryptocj520/crypto-trading-open
```

**项目角色**

```text
Conceptual strategy reference only
```

**冻结参考 SHA**

```text
620737399bfe3c331f9989fc77d631536f2e89df
```

其公开项目中，与本项目相关的核心概念包括：

- 分段/网格套利
- 价差扩大逐步建立仓位
- 价差收敛逐步减仓
- 历史“天然价差”/中位数
- `target - actual = delta`
- 多腿套利概念

### CJ 源码许可证边界

当前 GitHub 仓库元数据未声明开源许可证。

因此：

> **CJ 仓库只能作为“思想/行为参考”，禁止直接复制、翻译、移植其受版权保护的源码实现。**

Codex 必须：

- 根据本任务书重新独立实现算法。
- 不复制 CJ 函数、类、文件结构或大段注释。
- 不做逐行 Python→Rust 翻译。
- 只实现公开概念：grid / target inventory / spread convergence。

---

## 2.4 CJ 辅助仓库

CJ 账号下其他仓库只能作为辅助研究，不能自动提升为主规范，例如：

```text
https://github.com/cryptocj520/cross-exchange-arbitrage
https://github.com/cryptocj520/perp-dex-tools
https://github.com/cryptocj520/grid
https://github.com/cryptocj520/grid1.3
```

如果其中出现值得吸收的新功能，必须先做 `Upstream Change Report`。

---

## 2.5 上游更新治理

项目根目录必须创建：

```text
UPSTREAM_SOURCES.md
```

至少记录：

```text
source
repo
role
pinned_ref
license
last_reviewed_at
accepted_concepts
rejected_concepts
```

### 禁止自动追上游

Codex 不得因为：

- Nautilus 新 commit
- QuantGuy 新 commit
- CJ 新 commit

而自动修改生产行为。

上游更新流程固定为：

```text
Discover change
    ↓
Upstream Change Report
    ↓
Impact analysis
    ↓
GPT review
    ↓
ACCEPT / REJECT / DEFER
    ↓
独立 feature branch 实现
```

---

# 3. 项目最终目标

长期目标：

> **Multi-Venue Perp Delta-Neutral Arbitrage Engine**

最终可以发展到：

```text
多个 CEX / DEX
      ↓
统一盘口
      ↓
PairMatrix
      ↓
Opportunity Ranking
      ↓
Dynamic Fair Value
      ↓
Grid Target Inventory
      ↓
Global Inventory
      ↓
Capital Allocation
      ↓
1:N / N:M Execution Basket
      ↓
Rebalance / Funding Optimization
```

但长期目标不能反过来污染 V1。

---

# 4. V1 的唯一任务

V1 只证明：

1. **价格算得对**
2. **目标仓位算得对**
3. **1:1 两腿下得对**
4. **发生故障不会留下失控裸仓**
5. **任何交易都能解释、记录、回放**

收益最大化不是 V1 第一优先级。

优先级：

```text
Correctness
>
Risk Safety
>
Recoverability
>
Observability
>
Execution Quality
>
Profit Optimization
```

---

# 5. V1 Venue 范围

## 5.1 目标 Venue

优先使用 Nautilus 官方 adapter：

```text
Hyperliquid
├── Entropy / io HIP-3
└── trade.xyz / xyz HIP-3

Lighter
└── Mainnet
```

---

## 5.2 V1 symbol 范围

- 从 **1 个共同可交易 symbol** 开始。
- P1 连通性通过后再选具体 symbol。
- 不得为了“多一点机会”在 V1 同时开多个 symbol。

---

## 5.3 Adapter 原则

如果 Nautilus 官方 adapter 已支持：

> 必须优先使用官方 adapter。

禁止为了“感觉更灵活”：

- 自己重写 Hyperliquid adapter
- 自己重写 Lighter adapter
- 在 Strategy 直接调用 venue private REST/WS 绕过 Nautilus

如官方 adapter 存在真实缺口：

1. 先确认缺口。
2. 出 `Adapter Gap Report`。
3. 优先在项目边缘做最小 wrapper/extension。
4. 修改 Nautilus 上游源码只能作为最后手段，并需 GPT 单独批准。

---

# 6. V1 明确不做

Codex 不得自行加入：

- AI / LLM 交易决策
- K线预测
- maker 做市
- latency/HFT 抢跑
- 方向交易
- Spot-Perp
- Options
- 自动借贷
- 自动跨链
- 自动资金调拨
- Binance
- OKX
- Bybit
- 1:N live
- N:M live
- 自动 CapitalAllocator
- Generic Graph Optimizer
- Kafka
- Redis
- Microservices
- Kubernetes
- 华丽 Web dashboard
- 自动策略参数自学习
- 自动提高仓位上限
- 自动切入真钱实盘

---

# 7. 总体架构冻结

```text
                         Nautilus LiveNode
                               │
              ┌────────────────┼────────────────┐
              │                │                │
         Hyperliquid        Lighter         Future Venue
          Adapter           Adapter           Adapter
              │                │
        ┌─────┴─────┐          │
        │           │          │
   Entropy/io   trade.xyz   Lighter
        │           │          │
        └───────────┼──────────┘
                    │
                    ▼
             MarketNormalizer
                    │
                    ▼
                BookStore
                    │
                    ▼
               SpreadEngine
                    │
                    ▼
              FairValueModel
                    │
                    ▼
               RegimeFilter
                    │
                    ▼
            OpportunityModel
                    │
                    ▼
          GridInventoryModel
                    │
             TargetInventory
                    │
                    ▼
             InventoryManager
                    │
                    ▼
               RiskManager
                    │
                    ▼
        ExecutionBasketCoordinator
                    │
              legs[] (V1=2)
                    │
                    ▼
          ResidualDeltaHandler
                    │
                    ▼
             Reconciliation
                    │
                    ▼
       Recorder / Replay / PnL
```

---

# 8. 最核心架构规则

## 8.1 Strategy 不直接下交易所订单

Strategy 输出：

```text
TargetInventory / TargetExposure
```

Execution 层负责：

```text
Current vs Target
      ↓
ExecutionIntent
      ↓
Orders
```

---

## 8.2 Measurement 不决定仓位

禁止：

```text
SpreadEngine -> Order
FairValueModel -> Order
```

正确：

```text
Market
  ↓
Measurement
  ↓
GridInventoryModel
  ↓
Target
  ↓
Risk
  ↓
Execution
```

---

## 8.3 系统只有一个仓位大脑

禁止：

```text
QuantGuyStrategy
CJStrategy
```

正确：

```text
PerpArbitrageStrategy
  ├── SpreadEngine
  ├── FairValueModel
  ├── RegimeFilter
  └── GridInventoryModel
```

QuantGuy-inspired 只是测量层。

CJ-inspired `GridInventoryModel` 是唯一目标仓位策略。

---

## 8.4 90% Domain Logic 不依赖 Nautilus 类型

下列模块应优先为 pure Rust logic：

- SpreadEngine
- FairValueModel
- RegimeFilter
- GridInventoryModel
- Risk Rules
- PnL math
- Replay decision logic

只有边缘层：

```text
Nautilus Event -> Domain Type
Domain ExecutionIntent -> Nautilus Order
```

允许较强 Nautilus 耦合。

---

# 9. QuantGuy Measurement Layer

## 9.1 输入

- bid/ask levels
- depth
- exchange timestamp
- receive timestamp
- fee schedule
- estimated slippage
- funding（接口预留）
- venue health

## 9.2 输出

- executable long price
- executable short price
- executable notional
- gross spread bps
- fee bps
- slippage bps
- net executable edge
- midline
- deviation
- data quality
- validity

---

# 10. Net Executable Edge

V1 禁止使用 last price 产生 live order。

基本公式：

```text
NetEdge
=
ExecutableSpread
- Fees
- EstimatedSlippage
- FundingAdjustment
- RiskBuffer
```

V1 必须进入计算：

- bid/ask
- depth
- fee
- staleness
- basic slippage guard

Funding：

- V1 接口预留。
- 能可靠获取时记录。
- 是否强制纳入 gate 由 P3 验收决定。

---

# 11. FairValue / Midline

V1 不做 AI。

优先实现：

```text
Rolling Robust Median
```

并维护：

- minimum sample count
- warm-up
- window
- dispersion / volatility
- invalid state

可以支持：

```text
1h / 6h / 24h
```

但 live strategy 同一时间只启用配置指定的主窗口。

禁止：

- hard-code midline = 0
- 未 warm-up 就交易
- 极端样本直接拖动 midline

---

# 12. RegimeFilter

状态：

```text
NORMAL
DEGRADED
REDUCE_ONLY
HALTED
```

触发条件可包括：

- 极端 spread
- midline 快速漂移
- volatility spike
- stale feed
- venue unhealthy
- rejection spike
- timeout spike
- reconciliation mismatch
- latency spike

最重要原则：

> spread 越大不等于套利越好。

异常 regime 下禁止机械继续网格加仓。

---

# 13. CJ GridInventoryModel

CJ 思路必须抽象为：

```text
Deviation -> TargetInventory
```

而不是：

```text
if spread > X:
    open()
```

示意：

| Deviation | Target |
|---:|---:|
| 0 bps | 0% |
| 5 bps | 20% |
| 10 bps | 40% |
| 15 bps | 60% |
| 20 bps | 80% |
| 25 bps | 100% |

反向偏离按对称/配置规则处理。

---

## 13.1 Target - Actual = Delta

系统统一执行思想：

```text
desired target
-
actual inventory
=
required change
```

例如：

```text
Target = -60%
Actual = -40%

Delta = -20%
```

系统只增加 20%。

价差收缩：

```text
Target = -40%
Actual = -60%

Delta = +20%
```

系统自然减 20%。

---

# 14. Measurement 与 Grid 的冲突消除

## 14.1 增加新风险

必须：

```text
Measurement valid
AND NetEdge acceptable
AND Grid target requires more risk
AND RiskManager approves
```

---

## 14.2 降低风险

如果：

```text
Target risk < Current risk
```

Measurement layer 不得仅因为：

```text
exit edge 不好
```

就阻止必要减仓。

降低风险优先。

---

# 15. Domain Types

必须至少定义：

## 15.1 `VenueBook`

```text
venue_id
instrument_id
bids[]
asks[]
exchange_ts
receive_ts
age_ms
sequence/version
```

---

## 15.2 `SpreadSnapshot`

```text
symbol
long_venue
short_venue
executable_long_price
executable_short_price
executable_notional
gross_spread_bps
fee_bps
estimated_slippage_bps
funding_adjustment_bps
risk_buffer_bps
net_edge_bps
midline_bps
deviation_bps
timestamp
validity
```

---

## 15.3 `TargetInventory`

```text
symbol
pair_id
target_fraction
target_notional
direction
reason
model_version
decision_id
```

---

## 15.4 `ExecutionIntent`

从第一天 Basket 化：

```text
intent_id
decision_id
symbol
target_net_delta
max_residual_delta
max_slippage_bps
legs[]
created_at
expiry
risk_context
```

V1 强制：

```text
legs.len() == 2
```

禁止写死：

```text
leg_a
leg_b
```

---

## 15.5 `ExecutionLeg`

```text
venue
instrument
side
target_qty
target_notional
reduce_only
order_policy
price_guard
```

---

## 15.6 `DecisionRecord`

记录：

- input books
- spread
- midline
- deviation
- net edge
- regime
- current inventory
- target inventory
- risk decision
- execution intent
- orders
- fills
- latency
- residual
- pnl attribution

---

# 16. InventoryManager

数据模型从第一天按 GlobalInventory 组织：

```text
Symbol
├── Venue A position
├── Venue B position
├── Venue C position
└── net_delta
```

V1 只使用 pair view。

避免未来 V2 推翻结构。

---

# 17. RiskManager

Risk 权限高于 Strategy。

必须检查：

- feed freshness
- venue health
- account state freshness
- market depth
- max venue position
- max pair exposure
- max global delta
- outstanding intents
- outstanding orders
- reject counts
- timeout counts
- reconciliation status
- latency
- regime
- session loss
- kill state

可能输出：

```text
APPROVE
DENY
REDUCE_ONLY
FLATTEN_REQUIRED
HALT_REQUIRED
```

---

# 18. Kill State

至少：

```text
READY
PAUSE_NEW
REDUCE_ONLY
FLATTEN
HALT
```

所有状态变化必须记录：

```text
from
to
reason
timestamp
trigger
```

---

# 19. ExecutionBasketCoordinator

V1 最大技术重点。

标准流程：

```text
ExecutionIntent
      ↓
Freeze Decision Snapshot
      ↓
Recheck Feed
      ↓
Recheck NetEdge
      ↓
Risk Gate
      ↓
Submit legs
      ↓
Track states
      ↓
Compute realized residual
      ↓
Residual handler
      ↓
Reconcile
      ↓
Finalize
```

---

# 20. Execution 状态原则

禁止多个 bool 拼状态。

显式状态机，例如：

```text
Created
RiskApproved
Submitting
Submitted
PartiallyFilled
ResidualHedging
Reconciling
Completed
FailedSafe
Halted
```

所有状态转换：

- 有原因
- 可记录
- 可 replay
- 可测试

---

# 21. Unknown Order State

最危险规则之一：

> timeout != failed

如果下单请求超时：

```text
禁止直接重下
```

必须先：

```text
Query / reconcile venue truth
```

确定：

- 未收到
- 已接受
- 部分成交
- 全成交
- 无法确定

只有在幂等性/状态确认后才能继续动作。

---

# 22. ResidualDeltaHandler

例如：

```text
Entropy SELL = $500 filled
Lighter BUY  = $320 filled
```

Residual：

```text
$180
```

系统必须定义：

- tolerance
- retry budget
- hedge path
- reduce-original-leg fallback
- maximum unhedged duration
- escalation rule

所有补单属于原：

```text
DecisionId + IntentId
```

不产生无法关联的新交易。

---

# 23. Reconciliation

## 23.1 Startup Recovery

启动顺序：

```text
Start
 ↓
Connect venues
 ↓
Load instruments
 ↓
Sync account
 ↓
Sync open orders
 ↓
Sync positions
 ↓
Build GlobalInventory
 ↓
Resolve orphan/unknown intents
 ↓
Warm up books
 ↓
Warm up fair value
 ↓
Validate
 ↓
READY
```

READY 前禁止增加风险。

---

## 23.2 Venue truth 优先

当：

```text
local state != venue state
```

本地不能强行覆盖真实 venue。

先进入：

```text
PAUSE_NEW / REDUCE_ONLY
```

再 reconciliation。

---

# 24. Recorder

必须记录：

- normalized market events
- account events
- order/fill events
- decisions
- risk outputs
- intent transitions
- reconciliation
- kill state
- PnL data

记录系统不能阻塞 hot path。

---

# 25. Deterministic Replay

V1 必须。

目标：

> 实盘出现问题 → 拿相同事件数据 replay → 尽可能得到相同 DecisionRecord。

要求：

- strategy 不依赖不可控 wall-clock 随机性。
- 随机行为必须 seed。
- Replay 禁止真实下单。
- schema versioned。
- 两次相同 replay 应产生相同 decision sequence。

---

# 26. PnL Attribution

至少拆：

```text
Gross Spread PnL
- Fees
- Slippage
+/- Funding
- Emergency Hedge Cost
+/- Inventory Mark PnL
= Net PnL
```

数据不足：

```text
unknown
```

禁止伪造精确数字。

---

# 27. 代码组织建议

```text
src/
├── domain/
│   ├── ids.rs
│   ├── market.rs
│   ├── spread.rs
│   ├── opportunity.rs
│   ├── inventory.rs
│   ├── execution_intent.rs
│   └── risk.rs
│
├── models/
│   ├── spread_engine.rs
│   ├── fair_value.rs
│   ├── regime.rs
│   └── grid_inventory.rs
│
├── strategy/
│   └── perp_arbitrage.rs
│
├── market/
│   ├── normalizer.rs
│   ├── book_store.rs
│   └── nautilus_bridge.rs
│
├── risk/
│   ├── manager.rs
│   └── limits.rs
│
├── execution/
│   ├── coordinator.rs
│   ├── residual.rs
│   ├── state_machine.rs
│   └── nautilus_bridge.rs
│
├── reconciliation/
│   └── manager.rs
│
├── recording/
│   ├── recorder.rs
│   ├── replay.rs
│   └── schema.rs
│
├── pnl/
│   └── attribution.rs
│
├── config/
│   ├── schema.rs
│   └── validation.rs
│
└── app/
    └── live_node.rs
```

如果现有仓库已有合理结构，不要求为了匹配目录机械重构。

重点是职责边界。

---

# 28. Rust 代码规范

## 28.1 CI 基线

必须通过：

```text
cargo fmt --check
cargo clippy -- -D warnings
cargo test
```

后续阶段增加：

- integration tests
- replay tests
- feature-specific tests

---

## 28.2 `unwrap/expect`

核心生产代码：

- 默认禁止 `unwrap()`
- 默认禁止 `expect()`

允许：

- tests
- 确认逻辑不可达，且有明确 invariant 注释

---

## 28.3 Error Handling

禁止：

- silently ignore
- `let _ = risky_call()`
- timeout 默认成功/失败
- 吞掉 venue error

错误需：

- 类型化
- 带上下文
- 可观测
- 明确是否 retryable

---

## 28.4 数值规范

避免裸 `f64` 在整个系统乱传。

优先：

- Nautilus precision types
- fixed Decimal
- typed newtypes

推荐：

```text
Bps
Price
BaseQty
Notional
Delta
VenueId
PairId
DecisionId
IntentId
```

严格区分：

- bps
- percent
- price
- qty
- notional
- USD delta

---

## 28.5 并发

要求：

- message-driven 优先
- 少量共享 mutable state
- 每个 task 有 owner
- 每个 task 有 shutdown path
- 无无限 retry loop
- 无 hot path blocking I/O
- 无 `sleep()` 掩盖 race condition

---

## 28.6 Unsafe

V1 默认：

```text
禁止新增 unsafe
```

除非：

- upstream requirement
- benchmark 证明必要
- GPT Gate 特别批准

---

# 29. 配置规范

建议：

```text
config/live.toml
```

包含：

- venue
- symbol
- fee assumptions
- midline window
- grid
- max position
- max global delta
- slippage
- stale limits
- execution timeout
- retry
- risk
- logging
- recording

---

# 30. Secret 规范

Secrets 只允许：

- environment
- secret store
- OS protected configuration

禁止进入：

- Git
- TOML
- YAML
- logs
- replay
- test fixtures
- screenshots
- Stage Reports

---

# 31. Observability

结构化日志至少携带：

```text
timestamp
decision_id
intent_id
symbol
pair
venue
state
reason
net_edge
midline
deviation
target_inventory
current_inventory
residual_delta
```

Metrics 至少：

- feed age
- event latency
- decision latency
- submit latency
- fill latency
- rejects
- timeouts
- active intents
- residual delta
- reconcile mismatch
- PnL components

---

# 32. 测试规范

## 32.1 SpreadEngine

必须覆盖：

- normal spread
- no edge after fee
- insufficient depth
- stale book
- empty side
- precision
- extreme spread
- inverted/corrupt book

---

## 32.2 FairValue

必须覆盖：

- warmup
- median
- rolling eviction
- outlier
- missing samples
- regime shift
- stale sample rejection

---

## 32.3 GridInventory

必须覆盖：

- expansion increases target
- convergence reduces target
- positive/negative direction
- max position
- zero deviation
- measurement gate blocks added risk
- measurement gate does not block necessary reduction

---

## 32.4 Risk

每个 hard limit 至少一条：

```text
approve case
deny/escalation case
```

---

## 32.5 Execution

至少：

- both filled
- A filled / B rejected
- A partial / B full
- both partial
- timeout but venue filled
- duplicate events
- out-of-order event
- retry exhaustion
- residual hedge
- flatten escalation
- restart while active intent

---

## 32.6 Integration

必须模拟：

- disconnect
- reconnect
- stale feed
- delayed fill
- duplicate fill
- partial fill
- venue reject
- process restart
- reconciliation mismatch

---

## 32.7 Replay

固定 event stream：

- replay 1 = replay 2 decision sequence
- no live order
- deterministic model outputs

---

# 33. Codex 自主权限

Codex 可以：

- 阅读整个仓库
- 改代码
- 新增代码
- 新增测试
- 局部重构
- 修 bug
- 更新必要 docs
- 当前阶段内反复迭代

---

# 34. Codex 禁止事项

Codex 不得：

- 改项目目标
- 突破 V1 scope
- 自动跟 upstream
- fork/魔改 Nautilus
- 复制无许可证 CJ 源码
- 删除失败测试来让 CI 绿
- 用 stub 冒充关键功能完成
- 绕过 Risk
- 把 timeout 当失败直接重下
- 用 last price 做 live signal
- 默认 midline=0
- 无限加仓
- 自动切 live
- 自动提高风险额度
- 记录 secret

---

# 35. 小任务开发闭环

每个小任务：

```text
1 Inspect
2 Plan
3 Implement
4 Format
5 Lint
6 Unit Test
7 Relevant Integration Test
8 Self Review
9 Fix
10 Re-run
11 Report
```

报告：

```text
Task:
Changed:
Tests:
Known limitations:
Risks:
Next:
```

小阶段无需用户确认。

---

# 36. 大阶段 GPT Gate

每个大阶段结束后：

```text
Codex Stage Report
        ↓
GPT review
        ↓
PASS
PASS WITH FIXES
BLOCK
```

`PASS WITH FIXES`：

```text
Codex auto-fix
↓
tests
↓
short re-review
```

`BLOCK`：

禁止进入下一阶段。

---

# 37. GPT Gate 审核重点

每次至少检查：

1. 是否偏离本任务书。
2. 是否引入不必要复杂度。
3. Measurement 与 Grid 是否职责冲突。
4. Risk 是否可绕过。
5. Execution 是否存在状态漏洞。
6. Unknown order 是否安全。
7. Residual 是否可控。
8. 是否 deterministic replay。
9. 是否阻塞 V1.5/V2 扩展。
10. 测试是否真的覆盖风险。
11. 是否有 secret/license 问题。

---

# 38. P0 — Foundation

目标：

- Rust project
- Nautilus pin
- `UPSTREAM_SOURCES.md`
- typed config
- domain IDs/types
- module skeleton
- CI

必须额外完成：

### Upstream verification

Codex 在 P0：

1. 验证 Nautilus exact ref。
2. 记录当前 API/adapter compatibility。
3. 检查 QuantGuy pinned source。
4. 检查 CJ pinned source。
5. 确认 CJ no-license → concept-only。

验收：

- build
- fmt
- clippy
- test
- source pin
- no dependency on floating branch

### GPT Gate 0

重点：

- 架构
- upstream pin
- license
- scope

---

# 39. P1 — Connectivity

目标：

- HIP-3 discovery
- Entropy/io
- trade.xyz/xyz
- Lighter
- normalized books
- freshness/health

不交易。

验收：

- 目标 symbol 稳定盘口
- timestamps 可用
- reconnect 可恢复
- stale 可识别

### GPT Gate 1

检查：

- 是否优先用官方 Nautilus adapter
- 是否重复造轮子

---

# 40. P2 — Recorder & Replay

目标：

- normalized event recording
- execution/account event schema
- deterministic replay
- versioning

验收：

- 实时录一段
- replay 两次一致
- 无真实 order path

### GPT Gate 2

检查：

- deterministic
- schema
- sensitive data

---

# 41. P3 — Measurement

目标：

- SpreadEngine
- FairValueModel
- RegimeFilter
- OpportunityModel

验收：

- executable price
- depth
- fees
- stale gate
- midline
- warmup
- extreme spread safety

### GPT Gate 3

检查：

- math
- units
- false edge
- midline correctness

---

# 42. P4 — CJ Target Inventory

目标：

- GridInventoryModel
- target inventory
- current/target delta

验收：

- spread expansion → target up
- convergence → target down
- only one strategy brain
- QuantGuy layer no inventory control
- no orders emitted directly by Grid

### GPT Gate 4

检查策略职责边界。

---

# 43. P5 — Risk

目标：

- hard risk
- kill state
- session limits

验收：

- Risk cannot be bypassed
- PAUSE_NEW
- REDUCE_ONLY
- FLATTEN
- HALT

### GPT Gate 5

这是安全 Gate。

---

# 44. P6 — Execution Basket 1:1

目标：

- ExecutionIntent
- `legs[]`
- V1 legs=2
- explicit state machine
- residual handler
- idempotency

验收必须模拟通过：

- full fill
- partial
- reject
- unknown timeout
- duplicate
- reorder
- residual hedge
- retry exhaustion
- escalation

### GPT Gate 6

V1 最严格 Gate 之一。

---

# 45. P7 — Reconciliation & Recovery

目标：

- startup sync
- live reconciliation
- orphan intent recovery
- restart recovery

验收：

- READY 前不加风险
- venue truth > local
- mismatch → safe state
- restart reconstructs state

### GPT Gate 7

重点审故障恢复。

---

# 46. P8 — Tiny Live Readiness

目标：

- signal-only
- dry-run/shadow
- live checklist
- smallest position config
- safety report

要求连续运行一段约定时间。

不得自动真钱。

输出：

```text
LIVE_READINESS_REPORT.md
```

### GPT Gate 8

结果：

```text
GO
NO-GO
```

真钱仍需用户明确批准。

---

# 47. P9 — Tiny Live

前提：

- Gate 8 = GO
- 用户明确授权
- 最小账户/最小名义

禁止：

- 自动扩大仓位
- 自动提高 leverage
- 自动增加 symbols

验收：

- 无失控裸仓
- residual 有界
- replay 可解释
- reconciliation 稳定
- pnl attribution 合理
- fault 自动转安全状态

### GPT Gate 9

决定：

```text
V1 STABLE / NOT STABLE
```

---

# 48. V1.5 — 1:N

只有 V1 stable 后。

新增：

```text
DepthAllocator
Basket legs > 2
one-to-many hedge
```

例如：

```text
Entropy SELL $1000

Lighter BUY $600
trade.xyz BUY $400
```

V1 已 Basket 化，因此无需重写 ExecutionIntent。

---

# 49. V2 — 多所大乱斗 / N:M

新增：

- PairMatrix
- OpportunityScanner
- RouteOptimizer
- FundingModel
- GlobalInventoryOptimizer
- CapitalAllocator
- RebalanceEngine
- N:M Basket

未来 Venue：

- Binance
- OKX
- Bybit
- 更多 DEX/CEX

但 V2 不推翻：

- SpreadEngine
- FairValue
- Grid
- Risk
- Basket
- Recorder
- Replay

---

# 50. 多所扩展的架构要求

V1 禁止写：

```text
if entropy_lighter_spread ...
```

业务核心应使用：

```text
Opportunity {
  symbol,
  long_venue,
  short_venue,
  net_edge,
  midline,
  deviation,
  depth,
  health,
  score
}
```

因此未来：

```text
2 venues
→ 3
→ 6
→ 10
```

不会修改 strategy core。

---

# 51. Definition of Done

任务 Done 需要全部满足：

- scope 正确
- architecture 正确
- tests 绿
- fmt 绿
- clippy 绿
- no secret
- no silent failure
- no unresolved P0/P1 bug
- error paths covered
- state auditable
- docs updated
- Gate passed（如为阶段任务）

---

# 52. Bug Priority

## P0

- uncontrolled trading
- duplicate orders
- runaway delta
- secret leak
- corrupt reconciliation
- position truth corruption

阻断开发阶段。

## P1

- incorrect spread
- incorrect target
- partial-fill bug
- risk bypass
- restart bug
- unknown order mishandled

当前阶段修复。

## P2

- observability
- non-critical UX
- optimization

记录 backlog。

---

# 53. 性能边界

V1：

- correctness 优先
- hot path 不阻塞
- recorder async/buffered
- 避免无意义 clone
- latency metrics

禁止 premature optimization：

- lock-free everywhere
- custom allocator
- unsafe
- microservices

没有 benchmark 不做复杂优化。

---

# 54. Git / Branch / PR 规范

建议：

```text
main
develop (可选)
feature/p0-...
feature/p1-...
fix/...
```

每个大阶段：

- 独立 PR 或明确阶段性 PR。
- PR body 引用 taskbook stage。
- CI 全绿。
- Stage Report。
- GPT Gate。

禁止：

- 大量无关变化一个 PR。
- 混入格式化整个上游代码。
- 无测试大规模重构。

---

# 55. Commit 规范

推荐：

```text
p0: add typed domain identifiers
p1: normalize HIP-3 book events
p3: implement executable spread depth walk
p6: handle unknown two-leg order state
```

Commit 应尽量表达行为，不写：

```text
fix
update
changes
```

---

# 56. Documentation 规范

根目录建议至少：

```text
PROJECT_TASKBOOK.md
UPSTREAM_SOURCES.md
README.md
ARCHITECTURE.md
LIVE_READINESS_REPORT.md  # P8 后
```

阶段报告：

```text
docs/stages/P0_REPORT.md
...
```

---

# 57. License / IP 规则

必须遵守：

### Nautilus

按其上游 license 条款作为依赖使用。

### QuantGuy

MIT 仓库；允许合法参考，但项目仍优先独立实现。

### CJ

未声明 license：

```text
NO SOURCE COPYING
NO TRANSLATION
NO DERIVATIVE CODE PORT
CONCEPTUAL REIMPLEMENTATION ONLY
```

如果未来 CJ 发布明确 license：

- 先更新 `UPSTREAM_SOURCES.md`
- GPT 审核
- 再决定是否允许具体代码参考

---

# 58. 用户参与边界

项目目标是：

> 用户不成为“复制粘贴中间人”。

用户只参与：

1. 项目重大方向改变。
2. Secrets / account 配置。
3. 钱真实投入前批准。
4. 提高资金/仓位上限。
5. V1 → V1.5 → V2 决策。
6. 上游策略重大吸收决策（必要时）。

用户不负责：

- 修 Rust
- 复制代码
- 逐条贴报错
- 日常 CI
- 每个小 PR 审核

---

# 59. Codex Stage Report 模板

```markdown
# Stage Report

Stage:
Status:
Pinned project commit:

## Implemented
- ...

## Architecture
- ...

## Upstream refs used
- ...

## Tests
- fmt:
- clippy:
- unit:
- integration:
- replay:

## Fault scenarios tested
- ...

## Known limitations
- ...

## Risks
- ...

## Deviations from taskbook
- none / ...

## Security / secrets
- ...

## License/IP check
- ...

## Recommended next step
- ...

## GPT Gate package
- PR:
- diff:
- CI:
- relevant docs:
```

---

# 60. Upstream Change Report 模板

```markdown
# Upstream Change Report

Source:
Old ref:
New ref:

## What changed
- ...

## Why relevant
- ...

## Affected project modules
- ...

## Risk
- ...

## License impact
- ...

## Proposed action
- ACCEPT
- REJECT
- DEFER

## Implementation plan
- ...

## Tests required
- ...
```

---

# 61. GPT 大阶段审核模板

GPT 每个 Gate 输出：

```text
Gate:
Result: PASS / PASS WITH FIXES / BLOCK

Critical findings:
High findings:
Medium findings:

Architecture:
Strategy boundary:
Risk:
Execution:
Replay:
Testing:
Security:
Upstream/license:

Required fixes:
Optional improvements:

May proceed to next stage: YES / NO
```

---

# 62. Codex 第一次启动时的强制流程

Codex 进入仓库后：

### 第一步：不要写业务代码

先：

```text
1. 阅读 PROJECT_TASKBOOK.md
2. 扫描整个仓库
3. 检查 Cargo.toml / Cargo.lock
4. 检查已有代码
5. 检查已有 Nautilus dependency
6. 检查 CI
7. 检查 secrets 泄漏风险
8. 检查项目是否已有部分 P0-P9 实现
```

### 第二步：生成 Gap Analysis

输出：

```text
CURRENT_STATE.md
```

内容：

- 当前架构
- 与任务书差距
- 已完成阶段证据
- 未完成阶段
- 技术风险
- upstream pin 状态
- 建议从哪个 Gate 开始

### 第三步：只规划当前阶段

如果是空/早期仓库：

```text
只规划 P0
```

禁止提前写 P1-P9。

---

# 63. 给 Codex 的开工总指令

```text
You are the implementation agent for this repository.

PROJECT_TASKBOOK.md is the governing engineering contract.

Rules:
1. Read the entire taskbook before coding.
2. Inspect the repository and create a gap analysis.
3. Do not expand V1 scope.
4. Do not modify Nautilus core.
5. Pin upstream dependencies and references.
6. QuantGuy is measurement-only.
7. CJ concepts define the sole TargetInventory model; do not copy CJ source code.
8. Risk always outranks strategy.
9. Execution is Basket-shaped from day one, but V1 permits exactly two live legs.
10. Unknown order state must be reconciled, never guessed.
11. Do not enable real-money trading without explicit user authorization.
12. Work stage-by-stage.
13. For each small task: implement, test, self-review, fix, retest.
14. At the end of each major stage, stop and prepare the GPT Gate package.
15. Never mark a stage complete with failing tests or unresolved critical safety issues.

Start by producing CURRENT_STATE.md and a P0-only execution plan.
```

---

# 64. 最终架构摘要

本项目最关键的八条不可破坏原则：

1. **Nautilus 是基座，不 fork、不魔改。**
2. **QuantGuy 只做 Measurement。**
3. **CJ-inspired GridInventory 是唯一仓位策略。**
4. **Target - Actual = required trade。**
5. **Risk > Strategy > Profit。**
6. **Execution 从第一天 Basket 化，V1 live 仅 1:1。**
7. **Unknown state 必须 reconcile，不能猜。**
8. **先安全、可回放、可解释，再做 1:N / N:M / 多所大乱斗。**

---

# 65. 当前冻结上游清单

截至本任务书 V2.0：

```text
NautilusTrader
https://github.com/nautechsystems/nautilus_trader
reference snapshot:
e96a4ab8c8a5a7cae0ea6d37770d5ce2dee6db5c
dependency ref:
TO BE SELECTED AND PINNED IN P0

yourQuantGuy
https://github.com/your-quantguy/entropy-arb
reference:
aa0391471f6bf72f78c45801fb8117b7bf7e8c89
role:
measurement reference
license:
MIT

CJ
https://github.com/cryptocj520/crypto-trading-open
reference:
620737399bfe3c331f9989fc77d631536f2e89df
role:
strategy concept reference
license:
none declared in GitHub repository metadata
policy:
conceptual reimplementation only
```

---

# 66. 冻结声明

V1 开发开始后：

> **默认不再继续加入“看起来不错”的功能。**

新需求统一：

```text
Backlog
```

除非：

- 修正确性问题
- 修安全问题
- 解决当前阶段 blocker
- GPT Gate 要求

否则不得打断 P0→P9 流程。

---

**END OF PROJECT TASKBOOK V2.0**
