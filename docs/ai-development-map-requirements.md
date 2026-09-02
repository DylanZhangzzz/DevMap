# AI 开发地图与证据链平台——详细需求文档

> 工作名称：DevMap（暂定）  
> 文档版本：0.3  
> 状态：需求收口稿，可用于技术设计与 MVP 拆解  
> 主要读者：AI Agent、产品经理、研发负责人、平台工程师、安全与审计人员  
> 语言约定：本文件中的 MUST、SHOULD、MAY 分别表示“必须”“应该”“可以”。

---

## 1. 执行摘要

DevMap 是一个面向 AI Agent 时代的软件开发上下文与证据链平台。它把长期、复杂、多 PR、多开发者、多 Agent 的开发过程表示为一张可版本化、可验证、可查询、可视化的“开发地图”。

传统 Git 主要保存代码最终发生了什么变化，但通常不能低成本回答以下问题：

- 这段代码依据哪个需求、哪份文档、哪个版本、哪一句话开发？
- 某个实现方向是人类明确要求，还是 Agent 在岔路中自主选择？
- Agent 为什么选择这条路线，当时有哪些其他合理方案？
- 多个 PR、开发者和 Agent 如何共同实现同一个目标？
- 哪些测试、评审、日志或运行结果证明当前实现满足需求？
- 某项结论是否已经失效、被替代或与新证据矛盾？
- 新 Agent 如何在有限 token 预算内继续开发，而不是重新阅读全部聊天？
- PM 如何鸟瞰整个项目，发现阻塞、缺失证据、方向冲突和 Release 风险？

DevMap 的核心不是保存完整聊天，而是构建以下证据图：

```text
Requirement Trace ──► Activity ──► Commit / PR ──► Evidence
         │                 ▲
         │                 │
         └──► Agent Decision
                  │
                  ├── alternatives
                  ├── rationale
                  └── approval / verification
```

当 Agent 只是遵循明确要求时，系统只记录 Requirement Trace；只有 Agent 在多个有意义的方向中自主选择、补足重要空白或改变路线时，才记录 Agent Decision。

已有项目第一次启用 DevMap 时不进行历史决策回溯。系统以一个经确认的不可变 Common Ground 声明接入时的 source commit、当前有效需求、权限政策、开放路线和明确未知项，并建立 Adoption Boundary。DevMap 只保证该边界之后的开发路线和 Agent 决策证据链完整；边界之前的原因、作者归因和放弃方案不得根据代码或旧 diff 推测。

最终产品提供类似知识库 Graph View 的力导向拓扑：大型目标、Release 和 Epic 形成星系中心；Requirement、PR、Agent Decision、Commit、Test 和 Evidence 形成周边节点；跨 PR 依赖、共享架构和阻塞关系连接不同星系。PM 可以缩放、聚类、过滤、沿证据链回溯，并审查 Agent 的自主路线选择。

---

## 2. 产品原则

### 2.1 地图而非聊天归档

平台 MUST 保存“语义岔路、路线依据、实际行动和验证证据”，而不是把完整聊天当作默认上下文。

完整 transcript 可以作为原始审计证据保存，但：

- MUST 不进入 Agent 默认上下文；
- MUST 按需、按权限、按局部区间读取；
- MUST 与结构化结论分开；
- MUST 经过密钥、PII 和敏感数据扫描；
- MUST 可由结构化节点精确引用到 turn 或 tool-call 区间。

### 2.2 需求与 Agent 决策严格分离

平台 MUST 避免把“遵循人类要求”错误归因成“Agent 决策”。

- 明确的人类指令、需求文档条款、组织政策、已确立项目规则属于 Requirement Trace。
- Agent 在需求未指定的有意义岔路上作出的选择属于 Agent Decision。
- 人类对 Agent Decision 的批准或拒绝属于 Approval Event，不创建新的“人类决策”节点。
- Agent 对人类要求的归纳解释 MUST 与原始引用分字段保存，不能冒充原文。

### 2.3 证据优先

摘要、解释和置信度不是证据。系统 MUST 允许任何关键 claim 沿边回溯到：

- 原始需求来源；
- Agent Decision 的理由和替代方案；
- 实际 commit、PR、文件或 symbol；
- 测试、评审、benchmark、日志或部署结果；
- 产生这些记录的人、Agent、CI 或系统身份。

### 2.4 渐进式上下文

平台 MUST 支持分层读取：

```text
L0：Context Manifest       项目当前状态和索引
L1：Task Capsule           当前任务相关需求、决策、风险
L2：Evidence Objects       相关测试、代码、评审和依据
L3：Raw Artifacts          transcript、完整日志、大型产物
```

AI 默认只读取 L0 和必要的 L1，只有在验证、质疑、修改或审计时才向下展开。

### 2.5 Adoption Boundary 之后追加而非改写

从 Common Ground 生效时刻开始，权威记录 SHOULD 采用 append-only 事件和不可变内容寻址对象。

- 决策改变时创建新版本或 `supersedes` 边；
- 证据失效时添加失效事件；
- 不直接覆盖旧记录；
- 当前状态由事件归并得到；
- 任意 Release 都能重建当时的开发地图。

### 2.6 图数据与图显示分离

- Canonical JSON 图对象是权威事实；
- JSONL 是过程事件流；
- Graph DB 是可重建查询索引；
- HTML/WebGL 是交互式查看器；
- 节点坐标和折叠状态属于 View State，不属于证据。

### 2.7 系统必须回答的六个问题

对任意重要路线节点、Agent Decision、PR、Commit 或 Release，系统 MUST 能以结构化响应回答：

1. **为什么走这条路线？**  
   返回 Requirement basis、Agent rationale、前置 Evidence 和因果路径。
2. **这是人类指定还是 Agent 自主选择？**  
   返回 `requirement_trace` 或 `agent_decision` 分类，以及原始 actor 和 source。
3. **Agent 是否有权做这个决定？**  
   返回 delegated authority、policy、scope、Approval Event 和当前授权状态。
4. **有哪些方案被放弃？**  
   返回 alternatives、拒绝原因，以及它们是否真正实验过。
5. **哪项证据证明它有效？**  
   返回当前有效 Evidence、目标 commit/artifact digest、验证者和签名状态。
6. **这个决定是否已经被替代？**  
   返回 active status、`supersedes`/`contradicts` 路径、替代对象和生效时间。

回答 MUST 包含对象 ID 和可追溯路径，不能只有自然语言摘要。示例：

```json
{
  "target": "decision:7f13",
  "why": {
    "requirements": ["requirement:pay-r17"],
    "rationale": ["避免数据库与消息队列双写不一致"]
  },
  "route_origin": "agent_autonomous_choice",
  "authority": {
    "basis": "policy:architecture-change-v2",
    "status": "approved",
    "approval": "approval:pm-42-018"
  },
  "rejected_alternatives": ["alternative:direct-dual-write"],
  "supporting_evidence": ["evidence:test-t42"],
  "status": "verified",
  "superseded_by": null
}
```

### 2.8 参考架构：组合现有基础设施

DevMap 不应重新发明通用 tracing、provenance、attestation 和 Git 存储能力。推荐组合：

```text
Agent Hooks
   ↓ 捕获人类指令、Agent 决策、工具调用和代码活动
OpenTelemetry / OpenInference
   ↓ 提供运行 trace_id、span_id 和 AI-specific span semantics
Custom Decision + Claim Schema
   ↓ 补充 authority、alternatives、approval、supersedes
W3C PROV Graph
   ↓ 组织 Entity / Activity / Agent 及责任关系
独立 Context Repo 的普通 Git branches
   ↓ 隔离 mainline、活跃路线并绑定 PR、commit 和 snapshot
in-toto Attestation
   ↓ 为测试、构建、发布和其他软件声明提供可验证载体
Graph DB
   ↓ PM 鸟瞰、聚类、过滤、路径分析和交互查询
```

各层职责必须保持清晰：

#### Agent Hooks：捕获层

职责：

- 观察 session start/stop、prompt、tool call、文件修改、测试、commit 和 PR；
- 在重大语义岔路触发 Requirement/Decision 分类；
- 在 compaction、handoff 和异常退出前触发 checkpoint；
- 为每次捕获添加 Agent、human、repository、worktree 和 session 信息。

限制：Hook 只提供“发生了什么”的原始事件，不能单独证明这是一个有效 Decision，也不能替代授权和证据判断。

#### OpenTelemetry / OpenInference：运行关联层

OpenTelemetry 提供通用 distributed tracing、TraceId/SpanId 和上下文传播；OpenInference 在 OpenTelemetry 之上定义 LLM、tool、chain、retrieval 等 AI workload 的语义属性。因此所有 OpenInference trace 都可以作为 OTLP trace 处理，但 OpenInference 为 Agent 活动增加 AI-specific 含义。[OpenTelemetry Trace API](https://opentelemetry.io/docs/specs/otel/trace/api/) [OpenInference Specification](https://arize-ai.github.io/openinference/spec/)

DevMap 使用要求：

- 每个 session Activity SHOULD 关联 `trace_id`；
- 每个 tool/LLM/chain 操作 MAY 关联 `span_id`；
- 日志和事件 SHOULD 通过 trace context 关联；
- `trace_id` 只能作为运行关联标识，不能作为永久 Decision ID；
- tracing sampling、retention 或 backend 丢失不能破坏权威开发地图；
- 重要 Decision 和 Evidence 必须升格为 DevMap Canonical Object。

OpenTelemetry/OpenInference 解决“运行过程中发生了什么”，但不直接表达：Agent 是否有权决定、有哪些替代方案、人类是否批准、Decision 是否被替代。上述语义由自定义 Decision + Claim Schema 补充。

#### Custom Decision + Claim Schema：领域语义层

职责：

- 区分 Requirement Trace 与 Agent Decision；
- 表示 delegated authority；
- 表示 alternatives 和 rejected reasons；
- 表示 Approval Event；
- 表示 `supersedes`、`contradicts`、Waiver 和 Gate；
- 定义“当前有效”状态归并规则；
- 生成 AI 可低成本读取的 Manifest 和 Task Capsule。

这是 DevMap 最核心、最难被现有标准替代的产品层。

#### W3C PROV：通用证据图语义层

W3C PROV 的三个起点概念是 Entity、Activity 和 Agent，并支持 `used`、`wasGeneratedBy`、`wasDerivedFrom`、`wasAssociatedWith`、`wasAttributedTo` 和 `actedOnBehalfOf` 等关系，适合表达产物、过程、责任和委托。[W3C PROV-O](https://www.w3.org/TR/prov-o/)

DevMap 建议映射：

| DevMap 对象 | W3C PROV 映射 |
|---|---|
| Requirement、Decision、Commit、Artifact、Evidence | `prov:Entity` |
| Agent session、实现、测试、评审、构建、发布 | `prov:Activity` |
| Human、AI Agent、CI、Organization | `prov:Agent` |
| Agent 在人类授权下执行 | `prov:actedOnBehalfOf` 或 qualified delegation |
| Activity 使用 Requirement/Decision | `prov:used` |
| Activity 产生 Commit/Evidence | `prov:wasGeneratedBy` |
| 新 Decision 替代旧 Decision | `prov:wasRevisionOf` 加 DevMap `supersedes` |

DevMap MAY 导出 JSON-LD/PROV-O，但内部 Canonical JSON 不必直接采用 RDF。自定义 schema 仍需保留 authority、alternatives、approval 和 Gate 等领域字段。

#### Context Repo 普通 Branch：版本、路线与代码绑定层

每个 Source Repo 默认对应一个独立 Context Repo。Context Repo 只使用平台普遍支持的普通 Git branch，不使用 custom refs：

- `main` 保存经合并或确认的 Canonical Project Graph；
- `bootstrap/initial` 隔离 Common Ground 草稿，确认后合入 `main`；
- `route/pr-<number>` 保存对应开放 PR 的 provisional 路线；
- `route/branch-<route-id>` 保存尚无 PR 的已上传 branch 路线；
- Bot 是 Context Repo branch 的唯一写入者，普通成员通过标准仓库权限读取；
- source PR 合并后，Bot 将有效 Capsule 提升到 Context `main` 并归档路线；
- rebase、squash、force-push 和 merge 的映射以 Canonical Object 保存。

Context Repo commit 是权威图的版本锚点。普通 branch 解决跨平台同步、并发路线隔离、网页可见性和权限管理；它不承担大型 raw artifact 存储或大规模交互查询。

#### in-toto Attestation：可验证声明层

in-toto Attestation Framework 用于生成关于软件如何产生的可验证声明，并让消费者验证软件来源和供应链信任。[in-toto Attestation Framework](https://github.com/in-toto/attestation/tree/v1.0/)

DevMap SHOULD 将以下对象表示或导出为 attestation：

- 测试针对哪个 commit/artifact 运行；
- 构建使用了哪些 source 和参数；
- Release 由哪些 artifact 组成；
- 哪个 workload identity 产生声明；
- Gate 根据哪些已签名 Evidence 通过。

in-toto 负责“谁对哪个软件产物作出了可验证声明”，但不负责保存 Agent 完整路线理由和 alternatives；这些信息由 DevMap predicate 或关联 Decision Object 表达。

#### Graph DB：查询与交互层

职责：

- 邻域、最短路径和因果路径查询；
- 社区聚类和语义缩放；
- PM 过滤与鸟瞰；
- blocking path、stale Evidence 和 orphan Decision 查询；
- 时间切片和 Release 对比。

Graph DB MUST 能从 Context Repo 普通 branches、Canonical Objects 和 attestation 重建，不能成为唯一事实源。

### 2.9 现有项目的可复用能力

最值得复用的不是某个完整产品，而是不同项目已经验证的局部机制：

- **Entire**：复用 checkpoint、并发 session、worktree 隔离和 Git-backed session metadata 的思想。Entire 的架构区分 ephemeral/persistent checkpoint，并支持多个 session 关联同一 checkpoint，说明长期 Agent 工作可以与 Git 生命周期绑定。[Entire Sessions and Checkpoints](https://github.com/entireio/cli/blob/main/docs/architecture/sessions-and-checkpoints.md)
- **Git AI**：复用行级归因、prompt/session 与代码关联、跨 Agent 采集和 Git notes 绑定。Git AI 已把 line ranges 连接到 session、prompt、agent 和 model，但 DevMap 需要进一步表达路线授权、替代方案与有效状态。[Git AI](https://usegitai.com/docs/get-started)
- **Ponytail**：复用“极小常驻规则内核 + 薄宿主适配器 + Session/Subagent lifecycle 注入 + 规则漂移测试 + 真实 Agent Session benchmark”的执行保障模式。Ponytail 将核心行为放在共享 Skill/AGENTS 规则中，并用宿主 adapter 与 hook 传播到不同 Agent；DevMap 应借鉴其合规机制，而不是其极简编码人格。其公开 benchmark 的隔离方法和污染复盘值得复用，但单一仓库、模型和小样本结果不得直接泛化为 DevMap 指标。[Ponytail](https://github.com/DietrichGebert/ponytail) [Agent Portability](https://github.com/DietrichGebert/ponytail/blob/main/docs/agent-portability.md) [Agentic Benchmark](https://github.com/DietrichGebert/ponytail/blob/main/benchmarks/results/2026-06-18-agentic.md)
- **OpenTelemetry/OpenInference**：复用统一运行事件、trace/span、Agent/LLM/tool 语义和现有可观测基础设施；不把 runtime trace 当作永久决策模型。
- **W3C PROV**：复用 Entity/Activity/Agent、derivation、association 和 delegation 语义；用 DevMap schema 扩展软件开发领域概念。
- **in-toto**：复用可验证软件声明、subject/predicate 思路和签名生态；将测试、构建和发布证据提升为可验证 attestation。

DevMap 的差异化核心是把这些层组合成一条“从人类要求或 Agent 岔路选择，到代码、验证和 Release”的可导航证据链，并为未来 AI 提供 token-budgeted context retrieval。

---

## 3. 目标与非目标

### 3.1 产品目标

DevMap MUST：

1. 为已有项目建立经确认、不可变且可引用的 Common Ground。
2. 记录 Adoption Boundary 之后代码变更依据的明确需求来源。
3. 只在 Agent 遇到有意义岔路并自主选择时记录 Agent Decision。
4. 连接 Requirement、Decision、Activity、Commit、PR 与 Evidence。
5. 支持长期任务在 context compaction、暂停、换 Agent、换开发者后继续。
6. 支持多开发者、多 Agent、多 worktree、多 PR 并发开发。
7. 使用独立 Context Repo 的普通 branches 隔离 mainline 与并发路线。
8. 在 rebase、squash、merge、cherry-pick 后尽可能保持证据关系。
9. 自动检测缺失证据、越权决策、失效依据和未解决矛盾。
10. 为 PM 提供可缩放、聚类、过滤、可交互的项目拓扑。
11. 对敏感 transcript 和大型证据提供独立存储与权限。
12. 保持 Git-native、Agent-agnostic 和平台可迁移性。

### 3.2 非目标

DevMap 初期不以以下能力为目标：

- 记录 Agent 的全部内部推理；
- 把每个工具调用都升级成一条决策；
- 自动判定业务需求本身正确；
- 替代 Git、Issue Tracker、CI 或代码评审平台；
- 仅凭代码风格猜测哪些行由 AI 生成；
- 回溯或重建 Adoption Boundary 之前的 Agent 对话、路线理由、作者归因或放弃方案；
- 根据旧代码、旧 diff 或 commit message 推测未被明确记录的历史决策；
- 对已关闭的历史 PR 进行全量语义考古；
- 把 Graph DB 或向量数据库作为唯一事实源；
- 默认向所有仓库成员公开原始聊天；
- 让拓扑布局位置影响证据语义。

---

## 4. 用户与权限角色

### 4.1 开发者

需求：

- 查看某段代码的需求依据；
- 理解 Agent 为什么选择某个方向；
- 接手其他开发者或 Agent 的任务；
- 在 PR 中发现未解释的重要变化；
- 补充、纠正或批准 Agent Decision。

### 4.2 AI Agent

需求：

- 在任务开始时获得最小充分上下文；
- 知道人类明确要求与自主选择的边界；
- 在有意义岔路主动记录 Agent Decision；
- 在 commit、PR、handoff 和 compaction 前写入 checkpoint；
- 按需追溯证据，而不是读取完整历史。

### 4.3 PM / Tech Lead

需求：

- 从 Release、Epic 或 Requirement 鸟瞰多个 PR；
- 查看 Agent 自主决策及其影响范围；
- 发现阻塞、矛盾、遗漏和缺失验证；
- 批准或拒绝超出授权范围的 Agent Decision；
- 查看交付目标是否有完整证据闭环。

### 4.4 Reviewer / QA / SRE

需求：

- 连接需求、代码、测试和运行结果；
- 判断测试是否针对当前 commit；
- 标记证据失效、矛盾或不可复现；
- 为 Release Gate 提供签署证据。

### 4.5 安全与审计人员

需求：

- 追溯 Agent、开发者、CI 和审批者身份；
- 查看关键变更是否经过授权；
- 访问受控的原始证据；
- 验证对象 hash、签名、时间和来源。

---

## 5. 核心概念模型

### 5.1 Common Ground

Common Ground 是项目第一次启用 DevMap 时，由确定性仓库事实、明确的人类输入和稳定文档引用组成的不可变共同起点。它不是 Agent Decision，也不代表对接入前历史的解释。

MUST 包含：

- 稳定 ID；
- Source Repo 的稳定 repository ID 和 remote URI；
- default branch、baseline commit SHA 和 tree hash；
- 生效时间 `adopted_at`；
- 当前有效目标、Requirement Source 和 policy revision；
- 初始化时可见的开放 PR 与远程活跃 branch 的 Route Start；
- 已知 Blocker、Gate 和明确 Unknown；
- 创建者、确认者和确认事件；
- `pre_adoption_history: untracked` 声明。

自动初始化 MUST 只写入可验证事实和明确引用，不得生成关于历史动机、历史 Agent 身份或放弃方案的推断性叙事。Common Ground 草稿 MUST 经获得授权的人类确认后才能成为 Context `main` 的 Adoption Boundary。

示例：

```json
{
  "schema_version": "devmap/v1",
  "id": "common-ground:cg-001",
  "object_type": "common_ground",
  "source_repository": "repo:payment-service",
  "default_branch": "main",
  "baseline_commit": "72ac91e",
  "baseline_tree": "sha256:98a7",
  "adopted_at": "2026-08-26T10:00:00Z",
  "requirement_sources": ["docs/payment-spec.md@blob:8a71c2"],
  "active_routes": ["route:pr-184", "route:branch-7f13"],
  "unknowns": ["接入前路线理由未追踪"],
  "pre_adoption_history": "untracked",
  "confirmation": "approval:cg-001"
}
```

### 5.2 Requirement Trace

表示 Agent 或开发者遵循的明确要求。它不是 Agent Decision。

MUST 包含：

- 稳定 ID；
- 来源类型：仓库文档、Issue、聊天、外部规范、政策或人类输入；
- 来源 URI 或仓库路径；
- 文档 revision、Git blob SHA 或内容 hash；
- section、clause 或稳定锚点；
- 短原文引用；
- 独立的 normalized requirement；
- 适用范围；
- 关联 Activity、Commit、PR 和 Evidence。

示例：

```json
{
  "schema_version": "devmap/v1",
  "id": "requirement:pay-r17",
  "object_type": "requirement_trace",
  "source": {
    "type": "repository_document",
    "path": "docs/payment-spec.md",
    "revision": "git-blob:8a71c2",
    "section": "3.2 事件可靠性",
    "clause": "PAY-R17",
    "excerpt": "支付成功后，订单事件必须保证至少投递一次。"
  },
  "normalized_requirement": "支付成功事件必须具备至少一次投递保证",
  "scope": ["src/payment/**"],
  "status": "active"
}
```

### 5.3 Agent Decision

表示 Agent 在需求或既有强约定没有唯一规定路线时，自主作出的有意义选择。

MUST 包含：

- 决策陈述；
- 作出决策的 Agent、模型、session；
- 依据的 Requirement 或前置 Decision；
- 至少一项理由；
- 当时存在的主要替代方案及拒绝原因；
- 影响范围；
- 授权范围和审批状态；
- 关联 Activity、Commit 和 Evidence；
- 当前状态和 supersedes 关系。

当 Agent 主动采用一个能力有限但当前足够的简化方案时，还 MUST 记录：

- `operational_ceiling`：该方案明确的能力上限；
- `revisit_when`：必须重新评估或升级的可观察触发条件；
- 对应的升级方案作为 alternative，而不是无期限的“以后再说”。

Agent Decision 的状态：

```text
proposed
   ├── approved ──► active ──► verified
   │                   ├──► contradicted
   │                   └──► superseded
   └── rejected
```

Agent 在已授权范围内的战术决策 MAY 直接进入 `active`；超出授权范围的架构、schema、安全、协议、迁移和跨团队方向 MUST 先进入 `proposed`。

### 5.4 Activity

表示开发者或 Agent 实际进行的一段工作，例如实现、重构、迁移、测试、评审、调试或部署。

Activity MUST 区分：

- `proposed_by`：谁提出方向；
- `authorized_by`：谁或什么规则授权；
- `executed_by`：谁执行；
- `verified_by`：谁验证。

一个 Activity 可以由多个人和 Agent 共同执行，不能假设单一作者。

### 5.5 Evidence

可验证地支持或反驳 Requirement、Decision 或 Claim 的对象。

支持的 Evidence 类型 SHOULD 包括：

- automated test；
- manual QA；
- benchmark；
- code review；
- static analysis；
- build result；
- deployment result；
- runtime log；
- incident；
- document excerpt；
- reproducible command result；
- signed approval。

Evidence MUST 记录：

- 针对的对象；
- 产生者；
- 执行环境；
- 对应 commit 或 artifact digest；
- 结果；
- 内容 hash；
- 时间；
- 是否仍然 current；
- 原始产物位置和访问权限。

### 5.6 Checkpoint

表示长期任务在某个阶段的可恢复状态。

Checkpoint MUST 包含：

- 当前目标；
- 当前 branch、worktree 和 HEAD；
- active Requirements；
- active Agent Decisions；
- 已完成 Activity；
- 未完成事项；
- blockers；
- open clarification；
- 最近 Evidence；
- 下一步建议；
- 从上一个 checkpoint 开始的增量变化。

### 5.7 Clarification

当 Requirement 存在会影响结果的重要歧义时，Agent MUST 创建 Clarification，而不是静默生成 Agent Decision。

状态：

```text
open ──► answered ──► incorporated
  └──► withdrawn
```

回答内容应成为 Requirement Trace 的新版本或补充来源。

### 5.8 Approval Event

人类对 Agent Decision 的批准、拒绝、限制或补充。Approval Event 本身不是另一条 Decision。

MUST 记录：

- action：approve、reject、request_changes、limit_scope；
- actor；
- source turn 或 UI action；
- 时间；
- 适用 Decision；
- 限制或备注。

### 5.9 Waiver

对缺失证据、已知风险或 Gate 的临时例外。

Waiver MUST 包含责任人、理由、影响、有效期和撤销条件。过期 Waiver MUST 自动失效。

### 5.10 Gate 与 Release Snapshot

Gate 表示合并、发布或阶段完成所需条件。Release Snapshot 是某个发布时刻的稳定图根，可重建当时所有有效 Requirement、Decision、PR、Evidence 和 Waiver。

### 5.11 Capture Gap

Capture Gap 表示 Agent adapter、hook、subagent propagation 或本地 journal 在某个时间区间内不可用，导致运行事实可能缺失。它 MUST 记录已知起止时间、route/session、失败能力、检测来源、Capture Grade 变化和修复状态。

`capture_gap` 描述本地/Agent 捕获缺口；`context_gap` 描述远程代码已经上传但对应 Context Bundle 缺失或不完整。前者在代码上传后仍未补齐时 MUST 被远程 Integrity workflow 关联或升级为后者。Gap 只能被补充证据关闭，不能由自然语言声明“应该完整”而关闭。

---

## 6. 关系模型

平台 MUST 至少支持以下有向边：

| 关系 | 含义 |
|---|---|
| `starts_from` | Route、Activity 或 Project Graph 从 Common Ground / Route Start 开始 |
| `observed_at` | 对象在 Adoption Boundary 或某个 source commit 首次被观测 |
| `requires` | 目标或 Requirement 要求某项工作 |
| `based_on` | Agent Decision 基于某项 Requirement 或 Evidence |
| `implements` | Activity、PR 或 Commit 实现 Requirement/Decision |
| `produces` | Activity 产生 Commit、Artifact 或 Evidence |
| `verifies` | Evidence 验证 Requirement、Decision 或 Commit |
| `contradicts` | Evidence 或 Decision 与另一对象矛盾 |
| `blocks` | 对象阻塞 PR、Gate 或 Release |
| `supersedes` | 新对象替代旧对象 |
| `approves` | Approval Event 批准 Agent Decision |
| `rejects` | Approval Event 拒绝 Agent Decision |
| `depends_on` | PR、Decision、Requirement 或 Activity 的依赖 |
| `touches` | Activity 或 Commit 影响文件、symbol、schema 或服务 |
| `belongs_to` | 对象属于 Project、Epic、PR 或 Release |
| `gates` | Gate 控制 merge 或 release |
| `derived_from` | 摘要、索引或对象由其他证据派生 |

系统 MUST 允许存在矛盾边，不能由摘要模型擅自消解冲突。

---

## 7. Agent 决策记录边界

### 7.1 必须记录 Agent Decision 的情形

满足任意条件时，Agent MUST 记录：

1. 在两个或以上有意义的合理方案中作出选择；
2. 需求给出结果但没有规定重要实现方式；
3. 改变公共 API、schema、协议或数据迁移方式；
4. 引入或替换架构、依赖、安全或兼容策略；
5. 影响多个模块、PR、Agent、团队或 Release；
6. 偏离当前计划或已记录 Agent Decision；
7. 因测试、benchmark、review 或生产证据改变方向；
8. 引入技术债、临时 workaround 或未来义务；
9. 选择会影响运维、性能、成本、可靠性或隐私的路线；
10. 未来另一个 Agent 仅看代码 diff 可能合理地选择另一条路线。

### 7.2 不应记录 Agent Decision 的情形

以下行为 SHOULD 归入普通 Activity：

- 变量名、格式化和机械性重命名；
- 明确项目模式下的重复性实现；
- 普通文件读取和工具调用；
- 没有形成新认识的失败命令；
- 已有 Requirement 或 Agent Decision 唯一决定的实现；
- 临时思考、未采用猜测和无影响草案。

### 7.3 判定算法

Agent runtime SHOULD 执行以下分类：

```text
if 存在明确 Requirement 来源
   and 实现路线由 Requirement 或强制项目约定唯一确定:
    创建或关联 Requirement Trace
    不创建 Agent Decision

elif Requirement 存在影响结果的重要歧义:
    创建 Clarification
    停止超出安全范围的实现

elif 存在多个有意义路线且 Agent 选择其中一个:
    创建 Agent Decision

else:
    归入当前 Activity
```

### 7.4 误归因保护

平台 MUST 检查：

- 人类原文与 Agent normalized interpretation 分开；
- Agent 不得声称未发生的人类批准；
- Requirement Trace 必须具有可定位来源；
- Agent Decision 不得伪装成 Requirement；
- 对人类要求的变更必须经过 Clarification 或 Approval；
- 人类批准只改变 Agent Decision 状态，不改变其提出者。

---

## 8. 长周期复杂任务生命周期

### 8.1 Task Start

Agent MUST：

1. 读取 Context Manifest；
2. 识别当前目标、Requirement、Agent Decision、Blocker 和 Gate；
3. 检查当前 branch、worktree、HEAD 与地图 snapshot 是否一致；
4. 只加载当前任务邻域；
5. 创建 session Activity；
6. 对未解决矛盾或过期证据进行提示。

### 8.2 Development Loop

每次有意义开发循环：

```text
读取当前局部地图
        ↓
识别下一步要求
        ↓
是否出现语义岔路？
   ┌────┴────┐
   否         是
   │          │
关联需求     Requirement 是否明确？
   │       ┌──┴──┐
   │       否     是但未规定路线
   │       │           │
   │   Clarification  Agent Decision
   │                   │
   └────────► Activity ┘
                 ↓
            Commit / PR
                 ↓
              Evidence
                 ↓
             Checkpoint
```

### 8.3 自动 Checkpoint 触发

以下时机 MUST 触发：

- commit 前后；
- PR 创建或更新；
- Agent Decision 创建、批准、拒绝或 supersede；
- 测试阶段完成；
- Agent 被阻塞；
- 计划发生重要变化；
- handoff；
- context 即将 compaction；
- session 正常停止或异常恢复；
- 达到配置的最大未 checkpoint 变更量。

### 8.4 Handoff

新 Agent MUST 能通过一个 Task Capsule 恢复：

- 当前目标；
- 必须遵循的 Requirement；
- 当前有效 Agent Decision；
- 最近验证结果；
- 未解决问题；
- 工作目录状态；
- 建议下一步；
- 深层证据引用。

Task Capsule SHOULD 保持在可配置 token 预算内，默认建议不超过 8,000 tokens。

### 8.5 首次初始化与 Adoption Boundary

已有项目首次接入 MUST 使用以下流程：

```text
连接 Source Repo 和空 Context Repo
              ↓
固定 default branch HEAD、tree hash 和远程可见路线 HEAD
              ↓
在 bootstrap/initial 生成 Common Ground Draft
              ↓
授权人确认明确需求、权限政策、开放路线和 Unknown
              ↓
Bot 合入 Context main，产生 Adoption Boundary T0
              ↓
从 T0 后的成功上传事件开始完整捕获
```

初始化 MUST 不遍历全部历史进行 AI 语义回溯，也不得把旧 commit、旧 diff、代码结构或提交者身份解释为历史决策证据。初始化前已经存在的开放 PR 或远程 branch MUST 创建 Route Start，记录：

- stable route ID；
- 首次观测的 source branch、PR、merge-base 和 head SHA；
- 明确 Requirement Source；
- `pre_adoption_work: true`；
- `pre_adoption_decisions: unknown`；
- `capture_started_at: T0`。

Route Start 只说明“从哪里继续记录”，不说明“之前为什么走到这里”。初始化目标是分钟级建立共同起点，不以历史语义覆盖率作为完成条件。

---

## 9. PR 与合并后的证据完整性

### 9.1 系统职责边界

DevMap MUST 不干预开发者或 Agent 的 Git 工作策略：

- 不自动创建代码 commit；
- 不决定 commit 粒度；
- 不自动 push 代码；
- 不上传未提交代码；
- 不创建隐藏 WIP code commit；
- 不要求开发者按固定频率 commit 或 push；
- 不把尚未上传的本地代码状态展示到团队统一图。

开发者或 Agent 自己决定如何开发、是否 commit、何时 commit，以及何时上传。DevMap 的权威团队工作流从“远程系统确认上传成功”这一事实开始。

本地 Agent Hook MAY 在上传前记录 Requirement、Agent Decision、Activity 和 trace context，但这些信息只能停留在本地 pending journal。只有与成功上传的远程对象关联后，才进入统一 Project Graph。

### 9.2 上传事件是权威工作流起点

以下远程事件 MUST 触发 DevMap ingestion：

- Git branch push 或 ref update；
- PR 创建或 head 更新；
- Context Bundle 上传；
- Test/Build Evidence 上传；
- in-toto attestation 或等效声明上传；
- merge 完成；
- Release artifact 上传；
- force-push、branch rename 或 branch 删除。

权威触发 SHOULD 来自 Git host webhook、post-receive event、PR webhook 或 artifact service acknowledgement。客户端 `pre-push` MAY 准备 Context Bundle，但不能在 remote 确认前把 branch 标记为 published。

```text
Agent / Human decides to commit and push
                   ↓
            Git remote accepts
                   ↓
       authoritative upload event
                   ↓
          DevMap ingestion workflow
                   ↓
     unified Project Graph is updated
```

上传事件 MUST 使用 repository ID、remote ref、before SHA、after SHA 和 event ID 进行幂等处理，重复 webhook 不能生成重复节点。

### 9.3 统一 Project Graph 与 Branch 子图

每个项目 MUST 只有一张统一图和一个全局默认布局。不同 branch 不是不同成员视图，而是统一图中的 provisional workspace subgraph：

```text
Unified Project Graph
├── Mainline Graph
├── Published Branch A Subgraph
├── Published Branch B Subgraph
├── Open PR Subgraphs
└── Release / Gate State
```

branch A 的代码仍然属于 branch A，但上传成功后，DevMap MUST 从远程 ref、commit、diff 和 Context Bundle 生成 branch A 子图。其他成员不需要 checkout branch A，也能在统一地图中查看已上传的 Requirement、Agent Decision、Activity、Commit、Evidence、依赖和冲突。

每条 source 开发路线 MUST 映射到 Context Repo 中一条普通 route branch：

```text
source PR #184                  context route/pr-184
source feature/payment-v2      context route/branch-<stable-route-id>
source default branch          context main
```

有 PR 时以 PR number 作为稳定路线键；尚无 PR 时由系统生成 route ID，不能只把可重命名的 branch name 当作永久身份。所有 Context branch 由 Bot 单写，防止多 Agent 直接争用同一 branch。统一 Project Graph revision 由 Context `main` commit 与当前 active route branch heads 的有序集合共同标识。

如果代码已经上传但 Context Bundle 缺失，系统 MUST 创建 `context_gap`，不能自行推断路线原因：

```json
{
  "object_type": "context_gap",
  "branch": "feature/payment-outbox",
  "commit": "8f3a20e",
  "status": "missing_context",
  "missing": ["requirement_or_decision_basis"]
}
```

Branch 子图状态至少包括：

```text
published
in_review
changes_needed
approved
merged
superseded
abandoned
```

### 9.4 PR 创建

创建 PR 时系统 MUST 生成 provisional PR Capsule，包含：

- 该 PR 实现的 Requirement；
- 该 PR 引入的 Agent Decision；
- 参与开发者和 Agent；
- 影响文件、symbol、服务和 schema；
- 关联 commit；
- 当前 Evidence；
- 未解决 Clarification、冲突和 Blocker；
- 依赖的其他 PR。

### 9.5 PR 更新

每次成功 push SHOULD 由远程事件生成 delta，不重复复制完整 session。系统将 Git diff、Context Bundle 和新增 Evidence 归并得到当前 PR Graph State。

### 9.6 Pre-Merge Context Integrity Check

PR SHOULD 提供必需状态检查：

```text
Context Integrity
✓ 重要变更均有 Requirement Trace 或 Agent Decision
✓ Agent Decision 没有伪装成人类要求
✓ 高风险 Agent Decision 已获批准
✓ 测试证据绑定当前 commit
✓ 没有未解决 Requirement 矛盾
✕ 回滚测试失败，阻塞 Release Gate
```

检查项 MUST 包括：

1. 是否存在无法解释的重要 diff；
2. Requirement Source 是否可解析且版本明确；
3. Agent Decision 是否在授权范围；
4. 必需 Evidence 是否存在；
5. Evidence 是否对应当前 commit 或其可接受祖先；
6. 是否存在未解决 `contradicts`；
7. 是否引用已 superseded Requirement 或 Decision；
8. 跨 PR 依赖是否满足；
9. Waiver 是否有效；
10. 敏感证据是否符合权限政策。

### 9.7 Merge、Rebase 与 Squash

平台 MUST 尽可能保持原始证据映射：

- 保存 original commit SHA；
- 保存 merge commit SHA；
- 对 squash 保存 patch identity、tree hash、文件和 symbol 映射；
- 将原始 PR Capsule 连接到 merge commit；
- 不因源 commit 消失而删除历史 Decision 和 Evidence；
- 无法可靠映射时标记 `mapping_uncertain`，不能静默声称完整。

### 9.8 Post-Merge

合并后系统 MUST：

- 冻结 PR Capsule 版本；
- 验证 route branch head 与 source PR head、Capsule hash 和 merge SHA 的绑定；
- 由 Bot 将已采用的 route objects 提升到 Context `main`；
- 创建 merge snapshot；
- 重新计算跨 PR Gate；
- 将仅针对旧 commit 的 Evidence 标记 stale；
- 触发必要的集成测试或运行验证；
- 更新 Release 拓扑。

路线提升成功后，Bot SHOULD 在 Context `main` 写入 route closure，再删除或归档对应普通 route branch。删除 route branch 不得删除已经提升的 Decision、Evidence、PR Capsule 或 abandoned route 摘要。

### 9.9 Force Push、Branch 删除与历史保留

Force-push MUST 作为新事件处理：旧 commit 标记为 `detached_from_branch`，新历史标记为 `published`；旧证据链不得被物理删除。Branch 删除后，其子图状态变为 `merged` 或 `abandoned`，并从默认 active topology 折叠，但保留审计关系。

### 9.10 Release Snapshot

发布时 MUST 生成不可变 snapshot，至少包含：

- Git tree 或 release artifact digest；
- 所有 active Requirement；
- 所有影响 Release 的 Agent Decision；
- merged PR；
- 必需 Evidence；
- 未消除风险和有效 Waiver；
- Gate 结果；
- graph root hash。

---

## 10. Context Integrity 完整性算法

一个交付目标被认为 evidence-complete，当且仅当：

1. 每个 required Requirement 都有稳定来源；
2. 每个重要代码变化可以连接到 Requirement 或 Agent Decision；
3. 每个 Agent Decision 都有 basis、rationale、scope 和 actor；
4. 超出授权范围的 Agent Decision 已批准；
5. 每个 required Gate 都有当前有效 Evidence；
6. Evidence 对应目标 commit、artifact 或可接受祖先；
7. 不存在未解决的 blocking contradiction；
8. 不存在已过期的 Waiver；
9. 所有跨 PR required dependency 已满足；
10. graph root 和对象 hash 可以验证。

验证输出 MUST 是结构化的：

```json
{
  "target": "commit:72ac91e",
  "status": "blocked",
  "unexplained_changes": 0,
  "missing_requirement_sources": 0,
  "unauthorized_agent_decisions": 1,
  "missing_required_evidence": 1,
  "stale_evidence": 0,
  "unresolved_contradictions": 0,
  "expired_waivers": 0,
  "blocking_objects": [
    "decision:7f13",
    "evidence:test-t57"
  ]
}
```

---

## 11. 存储架构

### 11.1 总体结构

```text
Source Git Repository
├── AGENTS.md / project rules
└── .devmap/policy.yaml

Independent Context Git Repository
├── main
│   ├── common-ground/
│   ├── manifests/
│   ├── objects/
│   ├── events/
│   ├── capsules/
│   ├── snapshots/
│   ├── policies/
│   └── views/global.json
├── bootstrap/initial
├── route/pr-<number>
└── route/branch-<stable-route-id>

Encrypted Object Store
├── raw transcripts
├── full logs
├── test artifacts
└── large binaries

Derived Services
├── Graph index
├── search index
├── embedding cache
└── HTML/WebGL topology viewer
```

### 11.2 Canonical Graph Objects

权威对象 MUST 使用确定性 Canonical JSON，SHOULD 采用内容寻址。

要求：

- UTF-8；
- 禁止重复 key；
- 固定 schema version；
- 确定性字段与数字序列化；
- object ID 与内容 hash 可验证；
- 大对象使用外部 URI + digest；
- 对象更新创建新对象或事件。

不建议使用 YAML 作为权威证据对象，因为 YAML 类型和 canonical hash 存在歧义。YAML MAY 用于人类编辑的 policy 配置。

### 11.3 JSONL Event Stream

Agent runtime MAY 先追加 JSONL 事件：

```jsonl
{"event":"requirement_linked","id":"requirement:pay-r17"}
{"event":"agent_decision_created","id":"decision:7f13"}
{"event":"commit_linked","id":"commit:8f3a"}
{"event":"evidence_attached","id":"evidence:test-t42"}
```

系统 reducer 将事件归并为 Canonical Graph State。单个大型共享 JSONL 文件不适合多 Agent 并发；事件 SHOULD 按 session、Agent 或 shard 写入。

### 11.4 独立 Context Repo 与普通 Branch

MVP MUST 使用独立 Context Repo 作为权威结构化地图存储，并明确不使用 custom refs 或 Git Notes 作为核心存储。默认映射为一个 Source Repo 对应一个 Context Repo。

Branch 语义：

| Context branch | 内容 | 写入者 | 生命周期 |
|---|---|---|---|
| `main` | Common Ground、已合并 canonical facts、Release、policy、全局布局 | DevMap Bot | 长期 |
| `bootstrap/initial` | 首次 Common Ground 草稿 | DevMap Bot | 确认后合入并删除 |
| `route/pr-<number>` | 开放 PR 的 provisional Context Capsule 和增量事件 | DevMap Bot | merge/close 后归档 |
| `route/branch-<route-id>` | 尚无 PR 的已上传 branch 路线 | DevMap Bot | 建 PR 时关联或结束时归档 |

Context Repo MUST 使用普通仓库权限、branch protection 和 Bot 身份。开发者与 Agent 不直接写 Context branches；它们上传 Context Bundle，Bot 校验、归一化并提交，从而保证一个 branch 单写和可审计提交历史。

统一 Project Graph 由以下 revision tuple 确定：

```json
{
  "context_main": "a81c2f0",
  "active_routes": {
    "route:pr-184": "c17b902",
    "route:branch-7f13": "9aa420d"
  }
}
```

Reducer MUST 以 Context `main` 为 canonical base，叠加所有 active route branch，并把 open PR、Release 和 Gate 状态归并为同一 Project Graph。未被采用的 route 对象不得污染 canonical `main`；abandoned route 结束时只将必要的 closure、引用和审计摘要提升到 `main`。

`views/global.json` 保存唯一共享布局。系统不得创建成员专属持久化视图；个人 zoom、camera、临时 filter、hover 和 selected node 只存在当前浏览器 session，刷新后回到全局布局。

跨仓库 promotion 不是原子事务。Bot MUST 使用幂等两阶段协议：先验证不可变 Capsule 已绑定 source repository、PR/branch、head SHA 和内容 hash，再在 source merge 成功后提升到 Context `main` 并绑定 merge SHA。失败任务 MUST 可由 reconciliation job 重试，且不得在未验证时宣称 canonical promotion 完成。

### 11.5 Raw Artifact Storage

大型或敏感证据 MUST 存储在独立加密对象存储，图中只保留：

- URI；
- digest；
- media type；
- size；
- encryption key version；
- retention；
- sensitivity；
- ACL policy。

### 11.6 Graph DB 与索引

Graph DB、搜索和 embedding MUST 是可重建派生层，不能成为唯一事实源。

---

## 12. AI 优先的 Context Manifest

每个主要 snapshot MUST 具有小型 manifest：

```json
{
  "schema_version": "devmap/v1",
  "project": "payment-platform",
  "common_ground": "common-ground:cg-001",
  "adoption_boundary": "source:72ac91e",
  "kernel_version": "devmap-kernel/v1",
  "capture_grade": "A",
  "capture_gaps": [],
  "snapshot": "release:v2.8.0-rc3",
  "source_commit": "72ac91e",
  "active_roots": ["goal:pay-20"],
  "open_blockers": ["evidence:test-t57"],
  "pending_agent_decisions": ["decision:7f13"],
  "required_context": [
    "requirement:pay-r17",
    "decision:7f13"
  ],
  "graph_root_hash": "sha256:98a7"
}
```

Manifest SHOULD：

- 控制在约 500–1,000 tokens；
- 只包含当前 active、blocked、pending 和索引信息；
- 不展开 superseded 历史；
- 提供局部图查询入口；
- 标记生成时间、source commit 和 freshness；
- 始终标记 Common Ground 与 Adoption Boundary，避免把接入前状态误认为完整历史；
- 标记 Kernel version、Capture Grade 和未关闭 gap；
- 可以从权威对象重新生成。

---

## 13. Agent 集成与 Skill 要求

### 13.1 Canonical Capture Kernel

DevMap MUST 维护一份 Agent-neutral 的 Canonical Capture Kernel，作为所有 Agent adapter、Skill 和 hook 的唯一行为语义源。Kernel SHOULD 控制在约 500–800 tokens，只保留无法省略的记录判断和真实性不变量：

1. 明确人类指令、文档条款或强制规则只创建 Requirement Trace，不创建 Agent Decision；
2. 没有 materially different alternatives 的机械选择只记录 Activity；
3. Agent 自主选择有意义路线时，在实施前记录 Agent Decision；
4. Requirement 具有会改变结果的重要歧义时创建 Clarification；
5. 超出 delegated authority 的 Decision 保持 `proposed`，不得自行声称批准；
6. Evidence 必须绑定当前 commit 或 artifact digest；
7. Adoption Boundary 之前没有证据的历史保持 `unknown`；
8. task completion、handoff 和 compaction 前必须 validation/checkpoint。

Kernel MUST 不包含完整 schema、长篇示例、平台安装说明或 raw transcript。详细 schema 放在按需读取的 reference；确定性归一化、ID、hash 和 validation 放在 CLI/platform code。

任何风险策略等级、Agent 类型或 adapter 能力等级都不得降低以下真实性不变量：不得伪造人类原文、不得编造 Approval、不得推测接入前历史、不得把未绑定产物的结果称为 Evidence。

### 13.2 建议 Kernel 核心提示词

```md
# DevMap capture ladder

1. Explicit human/document/policy direction?
   Link the exact Requirement source. Do not create a Decision.
2. No materially different valid routes?
   Record Activity only.
3. Agent autonomously chooses a material route?
   Record Agent Decision before mutation: basis, alternatives, rationale,
   scope, authority; add ceiling and revisit trigger for bounded shortcuts.
4. Material ambiguity in the requirement?
   Request Clarification. Do not silently choose for the human.
5. Outside delegated authority?
   Keep proposed until an Approval Event exists.
6. After work, bind Activity, commit/PR and Evidence; checkpoint before
   handoff/compaction/completion. Pre-adoption unknowns remain unknown.
```

### 13.3 薄 Adapter 架构

各 Agent adapter MUST 只负责宿主协议转换、生命周期事件桥接、身份与能力声明，不得复制或重新解释 Decision 语义：

```text
Canonical Capture Kernel + Adapter Contract
                    ↓
       ┌────────────┼────────────┐
   Codex Adapter  Claude Adapter  Generic CLI/MCP
       ↓              ↓                 ↓
 session/subagent  host hooks      explicit tool calls
```

平台 SHOULD 从 Canonical Kernel 生成或组装宿主 Skill/Rule；无法生成时，CI MUST 检查 adapter copy 与 Kernel 的关键不变量一致。核心捕获和验证逻辑 MUST 位于可打包的 DevMap CLI/library 中，不能要求所有宿主都安装 Node.js；宿主 adapter MAY 使用其原生 runtime 调用 CLI。

每个 adapter MUST 在 session start 发布能力声明，至少包括：

```json
{
  "adapter": "codex",
  "adapter_version": "0.1.0",
  "kernel_version": "devmap-kernel/v1",
  "capabilities": {
    "session_start": true,
    "user_prompt": true,
    "pre_post_mutation": true,
    "test_result": true,
    "commit_mapping": true,
    "pre_push": true,
    "compaction": true,
    "subagent_start": true
  }
}
```

### 13.4 Capture Grade

系统 MUST 根据实际可观测能力而不是安装名称计算并展示 `capture_grade`：

| Grade | 最低能力 | 完整性声明 |
|---|---|---|
| A | prompt、Decision、mutation、Evidence、commit、subagent 全生命周期 | 可声明 full native capture，但仍需 Integrity Check |
| B | Decision、mutation、Evidence 和 commit 可关联，raw transcript 或部分 tool event 缺失 | structured complete / audit partial |
| C | 仅显式 CLI/MCP 调用和 Git diff coverage | assisted capture，不保证主动记录完整 |
| D | 仅 AGENTS/Skill 指令，无可靠 hook/tool acknowledgement | instruction only，禁止声称证据链完整 |

Capture Grade 是观测覆盖等级，不是可降低证据真实性的运行模式。高风险路径 MAY 要求最低 Grade；低于要求时必须显示 `capture_incomplete` 或阻塞 Gate，不能通过自然语言补足缺失事件。

### 13.5 父子 Agent 传播

父 Session 的 Kernel 和 route context 不能假定自动传播给子 Agent。支持 subagent lifecycle 的 adapter MUST 在 SubagentStart 注入：

- project ID、Common Ground ID 和 Adoption Boundary；
- route ID、source branch/PR、worktree 和 HEAD；
- parent Activity、parent trace ID 和 delegation relation；
- authority policy hash、Kernel version 和 Capture Grade；
- 最小必要 Requirement/Decision 邻域。

子 Agent MUST 拥有独立 actor/session ID，并通过 `acted_on_behalf_of`/delegation 边连接父 Agent。宿主不支持 SubagentStart 时，adapter MUST 降低 Capture Grade；父 Agent MUST 在 dispatch payload 显式传递同一最小上下文，且系统仍不得声称获得 Grade A。

### 13.6 激活范围与可见状态

Capture Kernel SHOULD 在任何可能修改代码、配置、文档、测试、构建或发布状态的 Agent session 自动启用。普通只读问答、简单解释或无状态变更任务 MAY 不启用 mutation capture，但仍应保留明确的只读 session 分类。

CLI 或宿主 UI SHOULD 始终提供简短状态：

```text
DEVMAP  route:pr-184  grade:A  pending:3  checkpoint:8m
```

发生 hook/adapter 缺失时 MUST 显示 `CAPTURE INCOMPLETE` 和原因；状态不可只写在 debug log 中。

### 13.7 Prompt 不能作为唯一保证

平台 MUST 使用工具 acknowledgement、lifecycle hook、diff coverage、adapter contract test 和 CI Gate 共同确保执行。单靠 Skill/AGENTS prompt 只能提高遵循概率，Capture Grade 最高只能为 D。

Adapter CI MUST 验证至少以下不变量：

- 明确人类要求不能被标成 Agent Decision；
- Agent 自主重大岔路必须包含 alternatives、rationale 和 authority；
- 接入前历史不能被推断；
- Evidence 必须绑定 commit/artifact digest；
- 高风险越权 Decision 必须保持 `proposed`；
- subagent 必须继承 route、policy 和 parent delegation；
- hook 缺失或失败必须降低 Grade 或产生 gap，而不是静默声称完整。

---

## 14. Agent 工具接口

MVP SHOULD 提供：

```text
context.open(target?)
context.link_requirement(source, scope)
context.record_decision(basis, alternatives, scope, ceiling?, revisit_when?)
context.request_clarification(question, impact)
context.record_activity(type, scope)
context.link_commit(commit, activity)
context.attach_evidence(target, artifact)
context.checkpoint(reason)
context.validate(target)
context.query(neighborhood, filters)
context.status()
```

工具 MUST 自动补充：

- actor identity；
- Agent、model 和 session；
- time；
- repository、branch、worktree 和 HEAD；
- source turn；
- content hash；
- schema version。

Agent SHOULD 不直接手工生成 object ID 或修改 canonical object。

---

## 15. 生命周期 Hook 与强制机制

### 15.1 Agent Hook

本地 Agent Hook 用于准备 Context Bundle，不负责决定 commit 或上传。SHOULD 支持：

- session start：声明 adapter capabilities/Capture Grade，并加载 Kernel、Common Ground、route、Manifest 和 authority policy；
- user prompt submit：捕获明确人类要求的 source turn，但不得仅凭 prompt 自动制造 Requirement 或 Decision；
- subagent start：传播最小 route context、parent delegation、authority policy hash 和 Kernel version；
- pre-mutation：对高影响范围判断是否存在 Requirement/Decision；
- post-mutation：建立 Activity 与 diff 关联；
- post-test：附加 Evidence；
- pre-commit：可选地提示 unexplained changes，但不得替 Agent 决定是否 commit；
- post-commit：在本地 pending journal 准备 commit mapping，不自动 push；
- pre-push：准备待上传 Context Bundle，但不把 push 尝试当作成功；
- pre-compaction：写 checkpoint；
- stop：finalize 当前 turn，不得冒充 session 结束；
- session end：finalize session。

Hook SHOULD 保持轻量、具有明确 timeout，并调用共享 DevMap CLI/library 完成归一化，不在各 adapter 内重复实现 Decision 判断。宿主报告 hook 非零退出、timeout、协议不兼容或能力缺失时，wrapper SHOULD 写入 `capture_gap`；如果本地无法写入，则远程 diff coverage MUST 将缺失升级为 `context_gap`。

状态接口 MUST 至少显示 active route、Kernel version、Capture Grade、pending event 数量、最近 checkpoint 和 gap 原因。状态显示失败不得被解释为 Capture 本身成功。

远程 Git/PR/Artifact Hook 是权威触发，MUST 支持：

- ref update accepted：创建或更新 published branch subgraph；
- PR created/updated：创建或更新 PR Capsule；
- context bundle received：关联 Requirement、Agent Decision 和 Activity；
- evidence/attestation received：更新验证关系；
- merge accepted：验证并将 provisional route 子图提升到 Context `main`；
- force-push：保留旧历史并切换 branch head；
- branch deleted：归档 branch 子图；
- release published：生成 Release Snapshot。

如果 push 失败或 remote 未确认，权威 Project Graph MUST 不发生 published 状态变化。

### 15.2 高影响路径策略

组织可以在 `.devmap/policy.yaml` 指定：

```yaml
high_impact_paths:
  - migrations/**
  - api/**
  - schemas/**
  - security/**
  - infra/**
  - public/**

required_evidence:
  migrations/**:
    - migration-test
    - rollback-test
  security/**:
    - security-review
    - static-analysis
```

### 15.3 Fail-Open 与 Fail-Closed

- 本地记录服务暂时不可用：SHOULD 先写入本地 pending queue，不阻塞普通编辑，同时显示 `CAPTURE INCOMPLETE`；
- adapter/hook 缺失或失败：MUST 降低 Capture Grade，并在可行时创建 `capture_gap`；
- subagent propagation 不可用：不得声称 Grade A；高风险子任务 MAY 禁止 dispatch 或要求显式 handoff capsule；
- Agent 未选择 commit 或 push：DevMap MUST 不采取任何自动上传动作；
- 远程代码上传成功但 Context Bundle 缺失：创建 `context_gap`，MAY 阻塞 merge，但不得编造依据；
- 高风险 commit 或 PR 无法验证：MUST 阻塞 merge；
- raw transcript 上传失败：MAY 不阻塞代码，但必须标记 audit incomplete；
- 人类 Requirement 被 Agent 越权修改：MUST 阻塞；
- 普通低风险 Activity 未完整标注：MAY warning。

---

## 16. 力导向拓扑图需求

### 16.1 总体形态

界面 SHOULD 类似知识库 Graph View：

- 力导向布局；
- 支持缩放和平移；
- 节点聚类形成“星系”；
- 中心节点大、邻接节点小；
- 关系线体现 Requirement、Decision、PR 和 Evidence；
- 选中节点后突出上下游证据链；
- 支持暗色和亮色主题；
- 大规模场景使用 WebGL 或等效高性能渲染。

### 16.2 语义缩放

```text
L0：Organization / Project / Release
L1：Epic / Requirement / Service / PR
L2：Agent Decision / Activity / Commit / Test
L3：File / Symbol / Evidence / Transcript Span
```

默认鸟瞰不应展示所有 Commit 和 File。用户缩放或展开 cluster 后才显示细节。

### 16.3 聚类

系统 SHOULD 支持按以下维度聚类：

- Project；
- Release；
- Epic；
- Requirement；
- PR；
- Service/Module；
- Developer；
- Agent；
- 时间窗口。

自动社区发现只能用于布局建议，不能改变权威关系。

### 16.4 视觉编码

建议：

- 节点大小：影响范围、连接数量或聚合节点规模；
- 节点颜色：当前选择的着色模式；
- 外圈：验证状态；
- 实线：active 关系；
- 虚线：proposed 或 pending；
- 灰色：superseded；
- 红色：blocked 或 contradicted；
- 高亮路径：当前关键证据链。

用户 MUST 能切换着色模式：

- 按对象类型；
- 按状态；
- 按 PR；
- 按开发者/Agent；
- 按 Release；
- 按证据完整性。

### 16.5 交互

PM MUST 能：

- 点击节点查看摘要；
- 查看直接邻居；
- 只显示上游原因；
- 只显示下游影响；
- 显示 Requirement 到 Release 的完整证据链；
- 隐藏机械性 Commit/File；
- 定位 blocking path；
- 展开 Agent Decision 的替代方案和理由；
- 打开原始 Requirement 或 Evidence；
- 批准、拒绝或限制 Agent Decision；
- 创建 Clarification；
- 查看有效 Waiver；
- 按状态、PR、Agent、Owner、时间过滤；
- 临时缩放、搜索和过滤，但不产生个人持久化视图；
- 查看统一图中的 Context mainline、published branch 和 PR route 子图；
- 分享 snapshot 链接。

### 16.6 时间视图

拓扑 SHOULD 支持时间滑杆：

```text
Task Start ── Development ── PR ── Merge ── Release ── Incident/Fix
```

用户可查看任一时刻：

- 哪些 Requirement 已知；
- 哪些 Agent Decision active；
- 哪些 PR 正在开发；
- 哪些 Evidence 已存在；
- 哪个 Gate 被阻塞；
- 地图如何分叉、回退和收敛。

### 16.7 稳定布局

力导向布局每次重新计算可能漂移，影响 PM 心智地图。因此：

- 自动布局 SHOULD 使用 deterministic seed；
- 项目 MUST 只有一个持久化全局布局；
- global pinned node 和 cluster position SHOULD 保存到 Context `main` 的 `views/global.json`；
- 所有具有项目图读取权限的成员 MUST 看到相同拓扑、节点状态和全局布局；
- 个人 camera、zoom、hover、selected node 和临时 filter 不得持久化为成员视图；
- 只有获得授权的 PM、Tech Lead 或 Maintainer 可以更新全局布局；
- 权威证据对象不得包含显示坐标；
- Release Snapshot MAY 保存推荐布局快照，但它必须可丢弃和重建。

### 16.8 本地预览与 `devmap view`

MVP MUST 提供轻量本地只读查看器：

```bash
devmap view
```

它不是常驻后台服务，而是 CLI 内嵌的临时 Web Viewer：

```text
devmap CLI process
├── Context Repo branches / Canonical JSON reader
├── in-memory graph index 或可重建 SQLite cache
├── embedded frontend assets
└── localhost HTTP server
        ↓
http://127.0.0.1:<random-port>/?token=<ephemeral-token>
```

第一版 MUST 遵守以下范围：

- 单进程；
- 默认只读；
- 只监听 `127.0.0.1`；
- 使用随机端口和临时访问 token；
- 前端资源内嵌，不依赖 CDN；
- 不要求 Node.js、Docker、Neo4j 或独立数据库；
- 不要求账号、组织和多用户权限系统；
- CLI 退出后服务器停止；
- 不启动系统常驻 daemon；
- 不默认暴露 raw transcript；
- 不允许浏览器任意读取仓库外文件。

`devmap view` SHOULD 在启动和配置的刷新时机使用标准 Git fetch 同步 Context Repo 的 `main` 与 `route/*` 普通 branches，由本地 reducer 重建同一 Project Graph。在相同 revision tuple 下，不同成员 MUST 得到相同语义图和全局布局；本地只读 Viewer 不创建成员专属持久化视图。

MVP 本地查看器只需支持：

1. 显示力导向拓扑；
2. 按对象类型、状态和 PR 过滤；
3. 点击节点查看结构化详情；
4. 高亮上游、下游和证据路径；
5. 查看当前 snapshot 和 source commit；
6. 在大型图中按需加载局部邻域。

本地数据量较小时，Viewer SHOULD 直接读取 Context Repo branches 和 Canonical Objects；数据量较大时，MAY 创建可删除、可重建的本地 SQLite 缓存：

```text
Context Repo / Canonical Objects    权威数据
              ↓
.devmap/cache.db                派生缓存
              ↓
devmap view                     只读查询
```

首版明确不包含：

- PM 写入和审批；
- Agent 实时推送；
- 多用户同步；
- 跨仓库服务端查询；
- Hosted Service；
- Graph DB 集群；
- 完整 transcript 在线浏览。

后续静态导出 SHOULD 与本地 Viewer 复用同一套前端：

```text
Shared Viewer Frontend
├── devmap view：从 localhost API 按需读取
└── devmap export：从内嵌 snapshot 离线读取
```

静态 HTML 是不可写的“地图快照”；`devmap view` 是可探索、按需加载但首版只读的本地地图。

---

## 17. PM 控制面

### 17.1 默认首页

PM 进入 Project 或 Release 时 SHOULD 看到：

- 项目拓扑；
- active Requirements；
- pending Agent Decisions；
- blocked PR；
- missing/stale Evidence；
- unresolved Contradictions；
- 有效和即将过期的 Waiver；
- Release Gate 状态。

### 17.2 Agent Decision 审查

详情页 MUST 同时展示：

- Decision statement；
- Requirement basis；
- Agent 和 session；
- alternatives；
- rationale；
- scope；
- affected PR/Service；
- current Evidence；
- approval history；
- downstream impact。

PM 可以：approve、reject、request changes、limit scope。

### 17.3 证据完整性检查

PM 点击 Requirement、PR 或 Release 后，系统 MUST 能回答：

- 哪些要求已实现但未验证？
- 哪些变化没有 Requirement 或 Agent Decision？
- 哪些 Agent Decision 未批准？
- 哪些测试针对旧 commit？
- 哪些旧 Decision 仍被新代码引用？
- 哪个最短 blocking path 阻止 Release？

---

## 18. 搜索与 AI 查询

平台 SHOULD 支持结构化和自然语言查询，例如：

```text
为什么支付写入使用 Outbox？
哪些 Agent Decision 影响 PR-143？
哪个 Requirement 要求兼容旧 API？
哪些 Release Gate 被回滚测试阻塞？
过去两周有哪些 Agent 自主修改了架构方向？
显示所有没有当前 Evidence 的 active Requirement。
```

查询响应 MUST 返回对象 ID 和证据路径，不能只返回无来源摘要。

---

## 19. 安全、隐私与信任

### 19.1 权限分层

至少支持：

- Graph metadata read；
- Requirement content read；
- Decision read/write；
- Approval；
- Raw transcript read；
- Sensitive evidence read；
- Policy administration；
- Audit export。

源码读取权限不应自动等于原始 transcript 读取权限。

### 19.2 敏感信息

写入 raw store 前 MUST：

- secret scanning；
- PII scanning；
- configurable redaction；
- sensitivity classification；
- retention assignment。

Requirement excerpt SHOULD 保持最短充分引用，避免复制完整受限文档。

### 19.3 完整性与身份

关键对象 SHOULD 支持：

- content hash；
- Git commit signing 或等效签名；
- Agent identity；
- human identity；
- CI workload identity；
- timestamp；
- approval authenticity；
- provenance-of-provenance。

### 19.4 删除与保留

需要同时满足 append-only 审计和隐私删除要求：

- 图中可以保留 tombstone 和 digest；
- 受限 raw content 可以按政策删除；
- 删除后 MUST 标记 evidence unavailable；
- 系统不能继续声称该证据可访问或完整。

---

## 20. 非功能需求

### 20.1 性能与规模

初始目标：

- 单项目支持至少 100,000 个图节点和 500,000 条边；
- 鸟瞰视图默认渲染聚合节点，单次可见节点 SHOULD 控制在 5,000 以内；
- 局部邻域查询 p95 SHOULD 小于 500ms；
- 远程 Manifest 获取 p95 SHOULD 小于 1s；
- 本地事件追加 SHOULD 小于 200ms，且不能显著阻塞 Agent；
- lifecycle hook p95 SHOULD 小于 200ms；超时 MUST fail visibly，不能无限等待 Agent；
- Canonical Capture Kernel SHOULD 控制在约 500–800 tokens；
- 大型 raw evidence 不得进入普通 clone 路径。

### 20.2 可用性

- 本地记录 SHOULD 支持离线；
- 远程不可用时写 pending queue；
- graph index 可从 Context Repo 普通 branches 和对象存储重建；
- 不得因视图服务故障损坏权威数据。

### 20.3 可移植性

- 核心 schema 不绑定特定 Agent；
- Canonical Capture Kernel 和 Adapter Contract 不绑定特定 Agent；
- 每个 adapter MUST 发布版本、能力矩阵和 contract-test 结果；
- 核心 CLI、Viewer 和 validation MUST 不依赖 Node.js，宿主 adapter MAY 使用宿主已有 runtime；
- 支持不同 Git host；
- 支持导出 Canonical JSON；
- MAY 导出 JSON-LD、GraphML、DOT、Mermaid、SQLite 或自包含 HTML；
- 导出格式不改变权威对象身份。

### 20.4 可观测性

平台 MUST 记录：

- capture success/failure；
- hook latency；
- pending queue size；
- integrity check duration；
- object store upload failures；
- graph index lag；
- Agent Decision capture rate；
- Capture Grade 分布和降级原因；
- subagent propagation success/failure；
- `capture_gap` / `context_gap` 数量；
- adapter Kernel-version drift；
- false-positive/false-negative feedback。

### 20.5 Agentic Evaluation Harness

DevMap MUST 使用真实 Agent session 而不是单轮 prompt 评估捕获效果。评测 SHOULD：

- 固定公开或内部测试仓库 revision、任务和期望 Decision/Requirement 标签；
- 对 no-DevMap、instruction-only、explicit-tool、full-hook adapter 建立对照组；
- 每个实验使用独立工作目录、独立进程和全新 Agent context；
- 隔离全局 plugin、Skill、AGENTS 和用户配置，防止 baseline 被 DevMap 污染；
- 同时覆盖主 Agent、subagent、compaction、handoff、rebase/squash 和 hook failure；
- 测量 Decision precision/recall、Requirement 误归因、Evidence 错绑、subagent 丢失、恢复成功率、token 开销和 hook latency；
- 执行至少一个安全/正确性 scorer，不能只统计记录数量；
- 分别报告不同模型、Agent host、仓库和任务类型，不把单一实验结果泛化成普遍结论。

---

## 21. 功能需求清单

### 21.1 Capture

- **FR-CAP-000**：首次接入 MUST 建立经确认的 Common Ground 与 Adoption Boundary。
- **FR-CAP-001**：系统 MUST 从仓库文档、Issue、聊天或人工输入创建 Requirement Trace。
- **FR-CAP-002**：Requirement Trace MUST 保存来源版本和稳定锚点。
- **FR-CAP-003**：系统 MUST 支持 Agent Decision 的创建、批准、拒绝和 supersede。
- **FR-CAP-004**：系统 MUST 阻止把 Agent interpretation 标记为人类原文。
- **FR-CAP-005**：系统 MUST 支持多 actor Activity。
- **FR-CAP-006**：系统 MUST 将 Evidence 绑定到 commit 或 artifact digest。
- **FR-CAP-007**：系统 MUST 在生命周期边界生成 Checkpoint。
- **FR-CAP-008**：系统 MUST 对歧义创建 Clarification。
- **FR-CAP-009**：系统 MUST 不根据接入前代码、diff 或 commit message 推测历史 Agent Decision。
- **FR-CAP-010**：接入时已存在的开放路线 MUST 创建 Route Start，并将接入前 Decision 状态标记为 `unknown`。

### 21.2 Git 与 PR

- **FR-GIT-001**：系统 MUST 将 graph root 关联到 source commit。
- **FR-GIT-002**：系统 MUST 使用独立 Context Repo 的普通 Git branches，不得以 custom refs 作为存储后端。
- **FR-GIT-003**：系统 SHOULD 在 rebase、squash 和 merge 后迁移映射。
- **FR-GIT-004**：系统 MUST 为 PR 生成 Capsule。
- **FR-GIT-005**：系统 MUST 提供 Context Integrity 检查。
- **FR-GIT-006**：系统 MUST 支持跨 PR Requirement 和 Decision 关系。
- **FR-GIT-007**：系统 MUST 为 Release 生成稳定 snapshot。
- **FR-GIT-008**：DevMap MUST 不自动创建代码 commit、push 代码或上传未提交代码。
- **FR-GIT-009**：权威 published 工作流 MUST 由远程成功上传事件触发。
- **FR-GIT-010**：每个已上传 branch MUST 作为统一 Project Graph 中的 provisional 子图显示。
- **FR-GIT-011**：远程代码已上传但 Context Bundle 缺失时，系统 MUST 创建 `context_gap`。
- **FR-GIT-012**：系统 MUST 幂等处理重复 push/webhook 事件。
- **FR-GIT-013**：Force-push 和 branch 删除不得物理删除已有证据链。
- **FR-GIT-014**：Context `main` MUST 仅包含已确认 Common Ground、已合并事实和必要 route closure。
- **FR-GIT-015**：每个开放 PR 或已上传 source branch MUST 映射到独立普通 route branch。
- **FR-GIT-016**：Context Repo branches MUST 由 Bot 单写，并采用标准仓库权限与 branch protection。
- **FR-GIT-017**：跨仓库 Capsule promotion MUST 幂等、可重试并绑定 source head SHA、merge SHA 与内容 hash。

### 21.3 Agent

- **FR-AGT-001**：Agent MUST 在任务开始读取 Manifest。
- **FR-AGT-002**：Agent MUST 在语义岔路进行 Requirement/Decision 分类。
- **FR-AGT-003**：Agent MUST 在重大自主选择前记录 Decision。
- **FR-AGT-004**：Agent MUST 在完成前调用 validation。
- **FR-AGT-005**：Agent MUST 在 compaction 和 handoff 前 checkpoint。
- **FR-AGT-006**：Agent MUST 按需读取原始证据，不得默认加载完整 transcript。
- **FR-AGT-007**：系统 MUST 以一份 Agent-neutral Canonical Capture Kernel 作为所有 adapter 的语义源。
- **FR-AGT-008**：Kernel SHOULD 控制在约 500–800 tokens，完整 schema 与 deterministic logic MUST 按需加载或由工具执行。
- **FR-AGT-009**：每个 adapter MUST 在 session start 声明 adapter、Kernel version、capabilities 和 Capture Grade。
- **FR-AGT-010**：支持 SubagentStart 的 adapter MUST 传播 project、Common Ground、route、authority、parent delegation 和最小任务邻域。
- **FR-AGT-011**：不支持可靠 subagent propagation、mutation acknowledgement 或 tool acknowledgement 时，系统 MUST 降低 Capture Grade。
- **FR-AGT-012**：Instruction-only adapter 的 Capture Grade 最高为 D，系统 MUST 不声称证据链完整。
- **FR-AGT-013**：hook/adapter 失败 MUST 可见，并在可行时创建 `capture_gap`；远程缺失最终 MUST 形成 `context_gap`。
- **FR-AGT-014**：Adapter CI MUST 校验 Kernel 不变量、宿主协议输出和规则漂移。
- **FR-AGT-015**：采用能力有限简化方案的 Agent Decision MUST 记录 `operational_ceiling` 和 `revisit_when`。

### 21.4 Graph UI

- **FR-UI-001**：系统 MUST 提供力导向拓扑视图。
- **FR-UI-002**：系统 MUST 支持语义缩放和聚类。
- **FR-UI-003**：系统 MUST 支持按对象、状态、PR、Actor 和时间过滤。
- **FR-UI-004**：系统 MUST 支持上下游证据链高亮。
- **FR-UI-005**：系统 MUST 支持 blocking path 定位。
- **FR-UI-006**：系统 MUST 支持 PM 审批 Agent Decision。
- **FR-UI-007**：系统 SHOULD 支持时间滑杆。
- **FR-UI-008**：系统 MUST 将 View State 与证据数据分开。
- **FR-UI-009**：MVP MUST 提供 `devmap view` 本地只读查看器。
- **FR-UI-010**：本地查看器 MUST 默认只监听 `127.0.0.1` 并使用临时 token。
- **FR-UI-011**：本地查看器 MUST 将前端资源打包在 CLI 中，不依赖外部 CDN 或独立前端运行时。
- **FR-UI-012**：本地查看器 MUST 在 CLI 退出时停止，不得默认安装常驻 daemon。
- **FR-UI-013**：本地查看器 SHOULD 直接读取 Git 对象，并 MAY 使用可重建 SQLite 缓存。
- **FR-UI-014**：静态 HTML 导出与本地查看器 SHOULD 复用同一前端和图数据接口抽象。
- **FR-UI-015**：项目 MUST 只有一个持久化全局布局，不得创建成员专属持久化视图。
- **FR-UI-016**：所有成员在相同 Project Graph revision 下 MUST 看到相同拓扑和 branch 状态。
- **FR-UI-017**：个人 zoom、camera、hover、selection 和临时 filter MUST 仅存在当前浏览器 session。
- **FR-UI-018**：CLI 或宿主 UI SHOULD 显示 active route、Kernel version、Capture Grade、pending 数量、最近 checkpoint 和 gap 状态。
- **FR-UI-019**：Capture Grade 不足或 hook 失败时 MUST 显示 `CAPTURE INCOMPLETE`，不能只写入 debug log。

### 21.5 Security

- **FR-SEC-001**：raw evidence MUST 加密存储。
- **FR-SEC-002**：系统 MUST 支持独立 raw transcript ACL。
- **FR-SEC-003**：系统 MUST 扫描 secrets 和 PII。
- **FR-SEC-004**：系统 MUST 支持对象完整性验证。
- **FR-SEC-005**：系统 MUST 记录审批身份和来源。

### 21.6 路线解释与标准复用

- **FR-EXP-001**：系统 MUST 回答某条路线为什么被选择，并返回 Requirement、Decision 和 Evidence 路径。
- **FR-EXP-002**：系统 MUST 区分人类明确指定路线与 Agent 自主选择路线。
- **FR-EXP-003**：系统 MUST 判断 Agent Decision 是否处于 delegated authority 内。
- **FR-EXP-004**：系统 MUST 返回 Agent Decision 当时放弃的主要替代方案和原因。
- **FR-EXP-005**：系统 MUST 返回证明路线有效的当前 Evidence 和 attestation 状态。
- **FR-EXP-006**：系统 MUST 返回 Decision 是否被 supersede、contradict 或 invalidate。
- **FR-EXP-007**：运行 Activity SHOULD 关联 OpenTelemetry `trace_id`，细粒度操作 MAY 关联 `span_id`。
- **FR-EXP-008**：OpenTelemetry/OpenInference trace 不得替代永久 Canonical Object ID。
- **FR-EXP-009**：Canonical Graph SHOULD 可映射或导出为 W3C PROV 关系。
- **FR-EXP-010**：测试、构建和发布 Evidence SHOULD 支持 in-toto attestation 或等效可验证声明。
- **FR-EXP-011**：Graph DB MUST 可以从 Context Repo 普通 branches 和 Canonical Objects 重建。

### 21.7 Agent 适配与评测

- **FR-EVAL-001**：系统 MUST 提供可重复的真实 Agent Session evaluation harness。
- **FR-EVAL-002**：评测 MUST 隔离全局 plugin、Skill、规则文件和历史 context，防止 baseline 污染。
- **FR-EVAL-003**：评测 MUST 包含 no-DevMap、instruction-only、explicit-tool 和 full-hook 对照组。
- **FR-EVAL-004**：评测 MUST 报告 Decision precision/recall、Requirement 误归因、Evidence 错绑和 subagent 丢失率。
- **FR-EVAL-005**：评测 SHOULD 报告恢复成功率、token 开销、hook latency 和不同 host/model 的结果。

---

## 22. MVP 范围

### Phase 1：Common Ground、本地 Capture 与只读地图

包含：

- Canonical JSON schema；
- Common Ground 与 Adoption Boundary；
- `bootstrap/initial` 到 Context `main` 的确认流程；
- session JSONL；
- Requirement Trace；
- Agent Decision；
- Activity、Commit、Evidence；
- Context Manifest；
- Canonical Capture Kernel 与 Adapter Contract；
- Codex reference adapter（Phase 1B 当前实现的有效等级为 D；缺少可观测 mutation、Evidence 关联和 commit mapping 时不得声称 Grade A）；
- Generic CLI/MCP fallback（Phase 1B 当前实现的有效等级为 D；配置成功与有效激活必须分开报告）；
- adapter capability handshake 和 Capture Grade；
- SessionStart/SubagentStart context propagation；
- `devmap status` / capture 状态显示；
- Kernel invariant 与 adapter contract tests；
- CLI/tool API；
- 独立 Context Repo；
- Context `main` 普通 branch reader；
- checkpoint 与 resume；
- 基础 validation；
- `devmap view` 本地只读服务器；
- 内嵌拓扑前端；
- 类型/状态/PR 过滤；
- 节点详情和上下游证据路径；
- 可重建本地 SQLite cache（数据量需要时）。

不包含：接入前历史回溯、完整企业权限、PM 写入审批、Agent 实时推送、多用户同步、自动社区聚类、大规模托管 UI、Graph DB 集群。

Phase 1 的本地 Capture 只准备 pending Context Bundle，不自动 commit 或 push；本地图不宣称未上传代码已经进入团队 Project Graph。

Phase 1 建议实现顺序：

1. Common Ground、Adoption Boundary、Canonical JSON 和 Capture Kernel；
2. Adapter Contract、Codex reference adapter、Generic CLI/MCP fallback 和 invariant tests；
3. Context Repo `main` 普通 branch reader；
4. `devmap status` 与 gap/Capture Grade 可视状态；
5. `devmap view` localhost 只读服务器；
6. 内嵌前端与力导向拓扑；
7. filter、node detail 和 evidence path；
8. 大图局部查询和可重建 SQLite cache；
9. `devmap export` 自包含静态 HTML；
10. 再进入团队 route branch、Bot 写入和托管能力。

### Phase 2：PR 与团队协作

包含：

- Git host 集成；
- remote ref/webhook 权威上传事件；
- Context Bundle ingestion；
- Bot 单写 Context Repo branches；
- 统一 Project Graph；
- `route/pr-*` 与 `route/branch-*` provisional subgraph；
- source merge 后向 Context `main` 的两阶段 promotion；
- reconciliation job；
- `context_gap` 检测；
- push/webhook 幂等处理；
- force-push 和 branch 删除归档；
- 唯一全局布局同步；
- PR Capsule；
- Context Integrity status check；
- merge/rebase/squash 映射；
- 多 Agent 并发；
- Claude Code 等其他 Grade A/B adapters；
- 跨 adapter capability matrix 和 contract-test dashboard；
- 真实 Agent Session evaluation harness；
- Approval Event；
- 团队 policy。

### Phase 3：PM 拓扑控制面

包含：

- 力导向 WebGL 图；
- semantic zoom；
- cluster；
- filters；
- evidence path；
- blocking path；
- timeline；
- Agent Decision 审查；
- Release Snapshot。

### Phase 4：企业能力

包含：

- 独立对象存储；
- 细粒度 ACL；
- secret/PII 管理；
- 审计导出；
- 签名与 attestations；
- 数据保留；
- 跨仓库图；
- 大规模查询和分析。

---

## 23. 验收场景

### AC-00：已有项目建立 Common Ground

Given 一个已经开发一半、此前未使用 DevMap 的项目，  
When 负责人连接 Source Repo 与 Context Repo 并确认 Common Ground，  
Then系统固定 default branch baseline SHA、当前 Requirement Source、policy、开放 PR 和远程 branch Route Start，  
And在 Context `main` 建立 Adoption Boundary，  
And不扫描全部历史来推测旧 Agent Decision、旧路线理由或放弃方案，  
And所有预先存在的路线标记 `pre_adoption_decisions: unknown`。

### AC-01：明确需求，不产生 Agent Decision

Given 文档明确要求“使用 PostgreSQL advisory lock”并具有稳定版本，  
When Agent 按文档实现，  
Then 系统只创建 Requirement Trace、Activity、Commit 和 Evidence，  
And 不创建 Agent Decision。

### AC-02：需求未指定路线，Agent 自主选择

Given Requirement 只规定“支付事件不得丢失”，  
When Agent 在双写与 Outbox 中选择 Outbox，  
Then 系统创建 Agent Decision，包含 alternatives、rationale、basis 和 scope。

### AC-03：需求有重大歧义

Given Requirement 对旧 API 兼容行为存在两种解释，  
When 两种解释会改变对外行为，  
Then Agent 创建 Clarification，  
And 不将自己的猜测标记为 Requirement。

### AC-04：多 Agent 并发

Given 两个 Agent 在两个 worktree 开发同一 Epic 的不同 PR，  
When 它们分别写入事件和对象，  
Then 不发生单文件写冲突，  
And 图中保留各自 Actor、Activity 和 Evidence。

### AC-05：Squash Merge

Given PR 包含多个带证据的原始 commit，  
When PR 被 squash merge，  
Then merge commit 仍可回溯到 PR Capsule、Agent Decision 和 Evidence，  
And 不确定映射明确标记。

### AC-06：Evidence 过期

Given 测试 Evidence 针对旧 commit，  
When相关代码在之后被修改，  
Then系统将 Evidence 标记 stale，  
And Gate 不再把它视为当前有效证据。

### AC-07：Agent 越权

Given Agent 未获授权修改公共 API，  
When Agent 创建相应 Decision，  
Then Decision 状态为 proposed，  
And PR Integrity Check 阻塞直至人类批准或撤回。

### AC-08：AI 低上下文恢复

Given 项目包含数万图对象和数百次 session，  
When 新 Agent 接手一个局部任务，  
Then默认只读取 Manifest 和相关 Task Capsule，  
And无需加载全部 transcript 即可定位 Requirement、Decision 和下一步。

### AC-09：PM 鸟瞰

Given 一个 Release 跨越多个 Epic、PR、开发者和 Agent，  
When PM 打开拓扑，  
Then可以看到聚类星系、阻塞节点和跨 PR 关系，  
And点击节点可展开完整证据链。

### AC-10：敏感 transcript

Given 用户有源码读取权限但没有 raw transcript 权限，  
When 用户查看 Requirement 或 Decision，  
Then可以看到授权摘要和 digest，  
And不能读取受限 transcript 内容。

### AC-11：轻量本地预览

Given 本地仓库已包含 DevMap Canonical Objects，  
When 用户运行 `devmap view`，  
Then CLI 在随机 localhost 端口启动只读 Viewer，  
And 不要求 Node.js、Docker、Graph DB 或账号系统，  
And 用户可以查看拓扑、过滤节点和展开证据路径，  
And CLI 退出后本地服务器立即停止，  
And 删除 `.devmap/cache.db` 不影响权威数据且可以重新构建。

### AC-12：只有成功上传触发权威工作流

Given Agent 在本地修改并自主决定是否 commit 和 push，  
When 代码尚未被远程接受，  
Then DevMap 不自动 commit、不自动 push，也不把本地代码显示为 published，  
When 远程 Git 确认 branch push 成功，  
Then DevMap 以 remote event 为权威触发 ingestion，  
And 在统一 Project Graph 中创建或更新对应 branch 子图。

### AC-13：代码上传但 Context 缺失

Given branch commit 已经成功上传，  
And 对应 Context Bundle 不存在，  
When DevMap 处理 remote ref update，  
Then系统创建 `context_gap`，  
And不得推断该路线是人类指定还是 Agent 自主决定。

### AC-14：统一全局图

Given branch A 和 branch B 都已上传且尚未合并，  
When 两名成员在相同 Project Graph revision 打开 `devmap view`，  
Then两人从 Context `main` 与相同 `route/*` heads 看到相同 mainline、branch 子图、状态、冲突和全局布局，  
And个人临时 zoom、camera 和 filter 不产生持久化成员视图。

### AC-15：子 Agent 继承路线和权限

Given 父 Agent 在 `route:pr-184` 上委派一个子任务，  
When adapter 接收到 SubagentStart，  
Then子 Agent 获得相同 Common Ground、route、authority policy hash、Kernel version 和最小 Requirement 邻域，  
And子 Agent 使用独立 actor/session ID，  
And其 Activity 与 Decision 通过 delegation 边连接父 Agent。

### AC-16：Instruction-only 不冒充完整捕获

Given 某 Agent 只读取 AGENTS/Skill 指令，没有 hook 或 tool acknowledgement，  
When session 开始并声明能力，  
Then系统将 Capture Grade 设为 D，  
And UI 显示 instruction-only，  
And Context Integrity 不得声称该路线已获得完整 native capture。

### AC-17：Hook 失败可见

Given mutation hook timeout 或返回不兼容协议，  
When adapter wrapper 检测失败，  
Then状态显示 `CAPTURE INCOMPLETE` 和失败原因，  
And Capture Grade 降级，  
And可行时创建 `capture_gap`；如果远程代码已经上传且 Bundle 不完整，则创建 `context_gap`。

### AC-18：Adapter 规则漂移被 CI 阻止

Given 某宿主 adapter 删除了“不得推测接入前历史”不变量，  
When Adapter Contract Test 运行，  
Then测试失败并阻止 adapter 发布，  
And报告 adapter、Kernel version 和缺失不变量。

---

## 24. 风险与缓解

### 24.1 记录过多导致图成为毛线球

缓解：

- 严格区分 Requirement 与 Agent Decision；
- 机械性操作归入 Activity；
- semantic zoom；
- 聚合 Commit/File；
- 默认只显示 active 和异常节点。

### 24.2 Agent 忘记记录

缓解：

- 极小 Canonical Capture Kernel；
- 专用工具；
- lifecycle hooks；
- SessionStart 与 SubagentStart 传播；
- Capture Grade 和可见状态；
- diff coverage 检查；
- PR required status；
- completion validation。

### 24.3 Agent 过度记录以满足合规

缓解：

- 明确 Decision 阈值；
- 相邻小修改聚合成 Activity；
- 对低价值 Decision 提供 reviewer feedback；
- 监控 false-positive rate。

### 24.4 AI 错误解释人类需求

缓解：

- 保留短原文、版本和锚点；
- normalized interpretation 单独字段；
- 高影响歧义必须 Clarification；
- 不允许 Agent 自行声称批准。

### 24.5 Context Branch 并发与跨仓库非原子性

缓解：

- 一个对象一个内容寻址 blob；
- 事件按 session/shard 写入；
- 一个普通 route branch 只对应一条开发路线；
- Context branches 由 Bot 单写；
- Context `main` 写入串行化；
- Capsule promotion 使用幂等两阶段协议；
- reconciliation job 重试 source merge 与 Context commit 之间的失败窗口。

### 24.6 隐私和仓库膨胀

缓解：

- raw artifact 外置；
- 最小引用；
- 独立 ACL；
- retention 和 redaction；
- 普通 clone 不获取大型证据。

### 24.7 拓扑位置不稳定

缓解：

- deterministic seed；
- 保存 View State；
- cluster pinning；
- 布局数据与证据分离。

### 24.8 Common Ground 被误认为完整历史

缓解：

- 图中始终显示 Adoption Boundary；
- Common Ground 固定 `pre_adoption_history: untracked`；
- 预先存在的开放路线固定 `pre_adoption_decisions: unknown`；
- 初始化不生成历史动机、作者归因或放弃方案；
- 路线解释查询跨越 T0 时必须返回 `history_untracked`，不能自然语言补全空白。

### 24.9 Agent Adapter 漂移或宿主能力不足

缓解：

- Canonical Capture Kernel 作为唯一语义源；
- adapter 保持薄，只转换宿主协议；
- session start capability handshake；
- Capture Grade 按实际能力计算；
- adapter contract/invariant tests；
- 不支持 subagent 或 mutation acknowledgement 时明确降级；
- instruction-only 禁止声称完整捕获。

### 24.10 常驻规则挤占 Agent 上下文

缓解：

- Kernel 控制在约 500–800 tokens；
- 完整 schema、示例和 policy matrix 按需读取；
- deterministic logic 放入 CLI/library；
- Session 只加载当前 route 的最小邻域；
- 评测 token overhead 与恢复成功率，而不是只统计生成对象数量。

---

## 25. 产品决策状态

### 25.1 已冻结决策

1. MVP 不使用 custom refs 或 Git Notes 作为核心存储。
2. 每个 Source Repo 默认对应一个独立 Context Repo。
3. Context Repo 使用普通 `main`、`bootstrap/initial` 和 `route/*` branches，并由 Bot 单写。
4. 项目只有一张统一图和一个持久化全局布局；不存在成员专属持久化视图。
5. DevMap 不决定是否 commit 或 push；远程成功上传事件触发权威工作流。
6. 已有项目通过 Common Ground 建立 Adoption Boundary，不回溯接入前历史决策。
7. 接入前已存在的开放路线只记录 Route Start，并明确历史决策未知。
8. Agent 合规层采用 Canonical Capture Kernel、薄 adapter、capability handshake 和 Capture Grade。
9. 父 Session 的记录规则不得假定自动传给子 Agent；支持时必须通过 SubagentStart 显式传播。
10. Capture Grade 只描述观测覆盖，不得降低证据真实性不变量。

### 25.2 待确认产品决策

以下事项需要在 MVP 设计阶段确认：

1. Agent Tactical Decision 是否默认无需人工批准，以及授权范围如何配置；
2. 哪些文件或 symbol 变化构成“重要 diff”；
3. Requirement 来源支持哪些外部系统及其版本锚点；
4. Context Integrity 默认 warning 与 blocking 的边界；
5. Release Snapshot 的签名方式；
6. raw transcript 默认保留周期；
7. 跨仓库 Requirement、Decision 和 Release 如何建立统一 ID；
8. 拓扑 UI 首版最大可见节点数；
9. 是否允许组织完全关闭 transcript 保存，只保留结构化证据；
10. PM Approval 是否必须在 DevMap 内完成，或接受 Git PR review/外部系统事件；
11. 开源仓库如何分享公开图，同时保护私有 Agent 会话。

---

## 26. 成功指标

产品上线后 SHOULD 观察：

- 新 Agent 恢复任务所需 token 和时间下降比例；
- “无法解释的重要 diff”数量；
- Requirement 到 Evidence 的覆盖率；
- Agent Decision 的批准、拒绝和 supersede 比例；
- 因 stale Evidence 导致的提前发现次数；
- PM 定位 Release blocker 的平均时间；
- 跨开发者/Agent handoff 成功率；
- Context Integrity false-positive 与 false-negative；
- Agent Decision capture precision/recall；
- 人类 Requirement 被误归因成 Agent Decision 的比例；
- subagent propagation 丢失率；
- Capture Grade 分布、降级和 gap 发生率；
- adapter contract-test 通过率与 Kernel drift 数量；
- lifecycle hook p50/p95 latency；
- instruction-only、explicit-tool 与 full-hook 的恢复成功率差异；
- raw transcript 默认读取频率；
- rebase/squash 后证据映射保持率。
- Common Ground 从初始化到确认的耗时；
- Adoption Boundary 之后重要变化的 Requirement/Decision 覆盖率。

成功不应以“记录了多少聊天”衡量，而应以“未来 Agent 能否用更小上下文准确继续开发，以及团队能否验证每个重要岔路”为核心指标。

---

## 27. 产品定义总结

DevMap 将软件开发表示为一张持续生长的可验证地图：

```text
Common Ground 固定首次共同起点与 Adoption Boundary
             ↓
人类要求定义目的地和明确路线
             ↓
Agent 在每个有意义岔路记录自主选择
             ↓
开发者与 Agent 通过 Activity 和 PR 向前推进
             ↓
测试、评审和运行结果形成验证地标
             ↓
Context Repo 普通 branches 隔离路线并固定地图版本
             ↓
AI 使用局部地图继续开发
PM 使用拓扑鸟瞰方向、阻塞与完整性
```

平台的核心价值不是保存“Agent 说过什么”，而是保存：

> 软件从经确认的共同起点出发，经过哪些由人类规定或由 Agent 自主选择的重要路线，最终由什么证据证明到达了正确结果；接入前没有证据的历史保持未知。
