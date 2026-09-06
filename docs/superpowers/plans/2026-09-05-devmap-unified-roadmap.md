# DevMap Unified Delivery Plan

**Goal:** 开发者通过地铁拓扑读懂开发状态，Agent 通过同一数据按授权推进 Git 工作。

**Architecture:** 保留 Git/任务采集和轨道布局，在共享模型中区分事实、意图、观测和操作证据。先修地图语义，再实现只评估的流程层，随后逐期开放本地写入、远端交付和目标队列。

**Tech Stack:** 现有 Rust、Cargo、serde、journal/fs2、本地 Git、HTML/CSS/JavaScript、Node 测试；首期远端适配以本项目实际使用的 GitHub 为目标，具体 API 接入在该阶段核对官方接口。

**Spec:** [统一产品方案](../specs/2026-09-05-devmap-unified-product-design.md)

**Status:** 用户已确认方向并授权阶段 1–2。实现、自测及创建事件采集的范围调整见 [阶段 1–2 开发记录](2026-09-05-devmap-unified-progress.md)。P3–P6 是后续计划，不是当前执行授权；下列新增路径是计划中的职责归属，不表示文件已经存在。后续执行每阶段前细化其接口与失败测试，不能直接照旧 Phase 1C.1 的全自动默认值执行。

## Global Constraints

- 最新明确用户要求优先；阶段 1–2 已获得开发授权，提交、推送、合并和插件安装独立于当前开发验收。
- 事实、意图、观测、执行结果分别建模。虚线仅表示明确的未来计划。
- 真实人类操作优先；不自动恢复被撤回内容，不自动清理资源。
- 分支名不是目录名，当前任务位置不是分支最新提交，计划起点不是已证明的创建点。
- 路线执行必须指定实体；UI 选择不能决定 Agent 的写入目录。
- 只做影响可读性或执行正确性的变更；不整体重写布局引擎。
- 修改行为时先以有意义的失败测试锁定问题；纯文档和低影响文案不新增镜像实现的测试。
- 一阶段验收通过后再进入下一阶段，不把计划中的进度显示为真实执行状态。

## P1 — 身份与事实契约

**Files:** `src/dock.rs`、`src/route_plan.rs`、`src/git_relationship.rs`、`src/mcp.rs`；测试 `tests/dock_model.rs`、`tests/route_plan.rs`、`tests/map_mcp.rs`。

**输入/输出：**现有 Git、任务、路线快照 → 同一 revision 下的身份、起点证据和独立状态；既有 schema 兼容读取，新增语义字段明确版本。

- [ ] 定义分支引用、worktree 路径、本任务上下文和选择状态的对应关系；补“目录 devmap-main 实际属于其他分支”用例。
- [ ] 分开 recorded creation、plan start、common ancestor；旧路线数据标为 plan start，不迁移成 creation。
- [ ] 保持修改、发布、包含、乘客完整性独立；覆盖 clean+included+unknown roster、included+dirty、同 HEAD 多 worktree。
- [ ] 为接入后的创建与任务迁移增加带来源的事件记录；没有事件的旧工作区保持未知。
- [ ] `view:agent` 与地图共享字段，测试同一实体和 revision 不产生不同状态。
- [ ] 运行 `cargo test --test dock_model --test route_plan --test map_mcp`，记录行为差异和兼容说明。

**退出条件：**数据能无歧义地区分上述概念；UI 无需猜测字段含义，旧历史未被伪造。

## P2 — 地图首屏与全图收口

**Files:** `assets/dock.html`、`assets/metro-core.js`、必要时 `src/dock_asset.rs`；测试 `tests/metro_core.cjs`、`tests/dock_renderer.cjs`、`tests/dock_ui_contract.rs`；固定数据 `tests/fixtures/metro/development-overview.json`（新增）。

**输入/输出：**P1 快照 → 相同拓扑的开发聚焦与全图两个视图；任务检查面板继续独立。

- [ ] 固定第 10 节场景，先写 main tip、真实分支身份、共享 HEAD、未知乘客的行为回归测试。
- [ ] 分支名作站台主身份；实际 main 标签挂到 main tip；Current 改成明确的本任务工作区标记。
- [ ] 创建点与共同祖先分别标注；移除暗示全局时间顺序的文案。未知不能画成虚构连线。
- [ ] 简化为真实线路、站台、乘客摘要、关键交付状态；旧保留工作区仅按可靠事实降权，风险/未知保持可发现。
- [ ] 修正虚线只表达计划，细灰线只表达关联；正式 PR、checks、队列未观测时不显示进行中。
- [ ] 保持选择、缩放和乘客展开不扰动几何；已有全图滚动可用就不加新的总览引擎。
- [ ] 运行 `node --test tests/metro_core.cjs tests/dock_renderer.cjs` 与 `cargo test --test dock_ui_contract`。
- [ ] 390/516/1280px 截图与交互试读，使用实际仓库再次核对目录/分支身份；记录五问回答结果。截图不含认证 token。

**退出条件：**五问可正确回答，交叉与换乘关系无歧义，无虚假无人/合入判断。达到这一点停止 UI 轮番美化。

## P3 — Agent 下一步评估，保持源 Git 只读

**Files:** 新增 `src/workflow.rs`、`src/workflow_policy.rs`、`tests/workflow_evaluation.rs`；修改 `src/lib.rs`、`src/mcp.rs`、`plugins/devmap/skills/live-worktree-dock/SKILL.md`。

**接口约定：**评估输入含明确 repository/route/worktree、snapshot revision、语义事件和宿主提供的授权引用；输出提案含 operation kind、前置事实、阻塞原因、授权与能力状态。评估不能修改源 Git、触发网络写入或伪造授权。

- [ ] 以规则表测试 start_work、milestone_ready、ready_for_review、delivery_ready 四类事件；无变更/已在合适 worktree 不建议重复创建或 commit。
- [ ] 测试完成聊天不等于完成路线、任务选择不改变执行工作区、检查不匹配 HEAD 不建议交付、仅有授权文字不视为可执行。
- [ ] 实现强类型提案；定义错误原因 `missing_target`、`state_unknown`、`ownership_unverified`、`checks_outdated`、`authorization_missing`、`capability_missing`。
- [ ] 增加 `devmap_evaluate_workflow`，把理由和阻塞同步到地图详情；继续保留“建议”与“已执行”的区别。
- [ ] 运行 `cargo test --test workflow_evaluation --test map_mcp`，在临时仓库比较评估前后 refs、索引、工作目录无变化。

**退出条件：**Agent 能解释正确的下一步和不执行的原因；没有写权限也能完整读取开发状态。

## P4 — 本地 Worktree / Commit 闭环

**Files:** 新增 `src/git_executor.rs`、`src/operation.rs`、`src/workflow_lock.rs`、`tests/workflow_local.rs`、`tests/operation_recovery.rs`；修改 `src/workflow.rs`、`src/journal.rs`、`src/events.rs`、`src/mcp.rs`、`src/lib.rs`。

**接口约定：**执行输入为 P3 提案、稳定 request ID、预期快照和有效授权引用；操作日志至少区分 prepared/executing/succeeded/failed/reconciliation_required。结果附实际 OID、工作区 ID 和核对证据。

- [ ] 先定义宿主如何传入已确认授权、存储范围与撤销；Agent 不能自行扩大授权。无可信授权接入时只保留评估模式。
- [ ] 临时仓库测试新建、复用、分支/目录冲突、相同 request ID 重试；同 ID 不同输入必须拒绝。
- [ ] 实现仓库协调锁、工作区写锁及固定获取顺序；测试双执行器争用与中断释放。外部修改后旧提案失效。
- [ ] 实现精确暂存与 commit 内容验证；混有未知暂存内容、同文件重叠改动、HEAD/index 漂移进入核对，不自动 stash/reset。
- [ ] 在执行前记日志，执行后读取 Git 核对；模拟“提交成功但返回丢失”，恢复不生成第二个提交。
- [ ] 开放 `devmap_execute_operation` 的本地枚举；由返回的真实创建事件和提交更新地图。
- [ ] 运行 `cargo test --test workflow_local --test operation_recovery --test workflow_evaluation`，核对用户工作区从未作为测试写入对象。

**退出条件：**两个 Agent 可并行开发、受控提交；幂等恢复不覆盖人类修改，地图显示真实新增站点。

## P5 — Push / PR 交付

**Files:** 新增 `src/remote_workflow.rs`、`tests/workflow_delivery.rs`；扩展 `src/git_executor.rs`、`src/operation.rs`、`src/workflow.rs`、`src/mcp.rs` 及 Skill。

**接口约定：**远端目标用具体 remote/repository/source ref/target ref 身份，不从当前浏览器猜测。操作证据包含远端 OID 或 PR ID、检查对应 OID 与观测时间。

- [ ] 用本地 bare remote 测试普通 push、远端领先、拒绝推送、断线后结果核对；不实现 force push。
- [ ] 用 GitHub 适配器测试夹具覆盖查找已有 PR、创建 Draft/正式 PR、更新 PR、权限不足、创建成功但响应丢失。
- [ ] 语义里程碑按项目策略触发 push；ready_for_review 才默认开正式 PR；不对每次文件保存/回复开 PR。
- [ ] 测试同源同目标开放 PR 复用；远端查询失败时保持 unknown，不直接重试创建。
- [ ] 运行 `cargo test --test workflow_delivery --test operation_recovery`；另在明确授权的测试仓库完成一次真实 API 集成核验。

**退出条件：**路线可以可靠交付到一个实际 PR；地图和 Agent 都能区分本地提交、发布、PR 与合入。

## P6 — 同目标串行交付

**Files:** 新增 `src/integration_queue.rs`、`tests/integration_queue.rs`；扩展 `src/remote_workflow.rs`、`src/operation.rs`、`src/workflow.rs`、`src/dock.rs`。

**接口约定：**队列键为目标仓库与完整 ref；候选绑定源 OID、目标 OID、检查证据、授权及显式依赖。地图只显示执行器实际队列，旧计划顺序仍叫计划。

- [ ] 测试两个就绪候选同目标只能一个进入合入；不同目标互不长期阻塞。
- [ ] 第一项成功后重新读取目标，重新验证第二项；旧 checks 不能自动沿用。
- [ ] 覆盖人类插队合并、人工回退、目标删除/移动、冲突、明确依赖与阻塞项跳过策略。
- [ ] 远端适配必须确认提交与合并条件；能力不足时返回 capability_missing，不用本地锁假装实现跨机器串行保证。
- [ ] 运行 `cargo test --test integration_queue --test workflow_delivery --test operation_recovery`，验收实际 merge/快进/squash 证据，不制造统一假 merge 节点。

**退出条件：**从路线开始到实际交付的状态有连续证据；人类改变现实之后系统反映并重新评估，而不反向修复人类操作。

## 发布与停止条件

- 每阶段结束给出变更、验收、未覆盖能力；不预先承诺自动 commit/push/merge 已经可用。
- 发布沿用用户确认的集成和插件更新流程；分别验证源码 HEAD、运行二进制与安装缓存版本。
- P2 通过后冻结地图的基本语义与布局；后续阶段只加入已实现的流程状态，除非实际试读发现新的具体问题。
- 不估算虚假的固定日程。P1–P2 是下一次开发范围，P3–P6 是有明确退出标准的后续交付路线；每阶段根据前一阶段证据细化实施步骤。

## 方案自查

- 覆盖：身份/起点=P1，地铁直观性=P2，Agent 读取与触发=P3，本地写入与恢复=P4，Push/PR=P5，依次合入=P6。
- 保留：现有采集、journal、布局与兼容接口；不另起整套产品或存储架构。
- 边界：源码未修改；授权、远端能力和安装验证都有明确交付门槛；旧文档中的“默认 controlled 即可写”不迁移为自动授权。
