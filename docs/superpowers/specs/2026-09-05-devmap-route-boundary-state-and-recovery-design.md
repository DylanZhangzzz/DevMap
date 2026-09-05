# DevMap 路线边界、人工操作与恢复规则

日期：2026-09-05

当前范围修订：以 [地图优先原则与对外接口建议](2026-09-05-devmap-map-first-and-public-interface-design.md) 为准。下文自动 gate、WithdrawalGuard、执行协调及其验收条目保留为扩展研究，不再作为地图核心的强制要求；人工操作优先、事实与计划区分、异常展示继续有效。

状态：设计细化稿，未实现。用户已明确“人工回退为准，异常回撤在地图报警”；其他规则是本轮提出的实施契约。

关联：[路线意图与合并队列优化方案](2026-09-05-devmap-route-intent-and-merge-queue-design.md)。本文细化并替代其中关于状态、候选更新、回退和异常处置的概括性规则；原有权限与安全约束继续有效。

## 1. 人工操作优先的准确含义

1. 已发生的 Git 变化先作为事实记录。已确认的人工 cherry-pick、revert、reset 或手工修改，成为其所作用范围内的新基线；自动化不得为了恢复旧计划而反向覆盖。
2. 本地操作只改变本地基线。只有从权威远端确认目标变化后，才更新远端集成事实；本地 main 回退不代表远端 main 已回退。
3. 人工决定可以改变预期，但不能把失败测试改成通过、把部分采用改成完整采用、把本地修改改成远端已发布。影响仍显示在图上。
4. 系统不得自动重新 merge、cherry-pick、revert-of-revert 或生成补丁，去恢复已经人工撤回的成果。恢复需要针对相关回退记录的新人工指令，重新建立候选及检查。
5. 停止自动覆盖不要求先确认操作者身份：来源未知的外部变化也保留，受影响动作进入对账；不能因“不知道是谁”而把它改回去。
6. 本规则描述如何响应已经发生的外部操作，不新增执行 destructive reset、force-push、删除工作区或绕过仓库权限的授权。

### 1.1 事实、操作者、意图分别取证

| 字段 | 可接受的依据 | 不足以单独证明 |
| --- | --- | --- |
| Git 状态 | 本地已解析对象/状态、权威远端查询 | UI 缓存、命令启动成功 |
| 人工操作 | 经认证的宿主用户动作、provider 审计事件，或有权限的用户对具体 SHA 变化的明确确认 | author/committer 名称、提交标题、分支命名 |
| 撤回范围与原因 | 人工指令、捕获到的操作参数和结果映射、明确成果关联 | 仅凭删行数或 AI 对 diff 的解读 |

事件保存 actor_provenance 与 intent_provenance；两者可以不同。Agent 执行用户要求的回退可有人工意图与 Agent 执行者，不伪装成人亲自操作。身份不明显示“外部变化，来源待确认”。

## 2. 必须分别保存的状态

### 2.1 工作与交付

- Route 工作状态：`planned / active / ready / abandoned`。完成判定绑定计划版本与代码版本。
- Delivery 批次：固定 route_id、plan_revision、source_sha、目标仓库/ref、交付范围。一个 Route 可有多个显式批次。
- QueueEntry 状态：`queued / validating / integrating / reconciliation_required / confirmed / cancelled / superseded / no_change`。
- 尚未请求交付以无 QueueEntry 表示；`not_requested` 仅是 UI 派生标签。
- 阻塞原因是列表，不另造与上述状态竞争的 `blocked` 状态。条目有阻塞原因时不可调度。
- 发布状态继续区分 local、remote_confirmed、context_published；Context 发布失败不撤销已确认的 Git 集成事实。

### 2.2 历史集成和当前成果效力

IntegrationRecord 一旦 confirmed，不因后续回退改成“从未合并”。另设 OutcomeAssessment：

| 状态 | 含义 |
| --- | --- |
| present | 指定成果在所评估目标 SHA 上有足够证据支持 |
| partial | 仅部分明确范围有采用证据 |
| withdrawn | 对指定范围存在明确撤回记录；不泛指所有功能都消失 |
| unknown | 当前效力无法确定或相关证据已过期 |

每项判断必须保存 assessed_target_sha、scope、evidence_refs、assessment_kind 与 coverage。commit 可达性、patch 对应、测试和人工验收是不同证据，不能互相冒充。

例如：A 曾合入 main，随后被 revert。历史仍为“已合入”；当前成果标为“已撤回”。不能因 A 仍是 main 的祖先而显示功能仍有效。

## 3. 完成条件与交付快照

每项完成条件保存 condition_id、描述、验证方式（check/manual_acceptance）、验收主体或政策、必需性和版本。Agent 可提出条件；不能自行删去失败的必需条件。确定性项目模板可按既有政策直接采用。

`ready` 要求该计划版本所有必需条件均有有效通过/验收证据。没有完成条件时保持 active 并显示“完成条件未定义”；只有测试通过但人工业务验收尚未完成时不进入 ready。变更需求后，受影响验收必须重新取得。

默认交付包含该路线当前所有归属明确且已提交的工作。工作区有未提交修改时不默认提交旧 HEAD；只有明确指定“仅交付 SHA X”的指令才允许旧快照交付，UI 同时显示仍有未交付工作。

每个路线、同一目标，最多一个非终态队列条目。建立重复请求时返回既有条目，不重复排队。

排队后源 HEAD 变化：旧条目 superseded，新工作回到 active。新版本重新满足条件后，若持续交付授权仍有效，自动创建新批次并排队；否则只显示可提交建议。不得静默把正在验证的 source_sha 替换掉。

显式固定 SHA 的交付可在该分支继续开发后保留；仅当授权明确绑定固定 SHA 且相关依赖、目标和检查仍有效。一般的分支交付不适用此例外。

## 4. 状态转换表

### 4.1 Route

| 当前状态 | 事件与前提 | 下一状态 / 结果 |
| --- | --- | --- |
| planned | 开始已获授权的工作 | active |
| active | 当前版本所有必需条件满足 | ready；记录完成证据快照 |
| ready | 新代码、完成条件变更，或所依据证据被撤销 | active；旧证据保留但不用于当前版本 |
| planned/active/ready | 人工放弃 | abandoned；请求取消未完成交付，保留已有代码 |
| abandoned | 明确恢复工作 | active；保留原 route_id，新增计划版本，不自动恢复旧队列 |

集成确认不直接修改 Route 工作状态；已就绪的批次与后续 active 工作可以共存。后续成果被回退也不重开旧任务，不自动产生“修复回退”工作。

### 4.2 QueueEntry

| 当前状态 | 事件 / 前提 | 下一状态与动作 |
| --- | --- | --- |
| 无条目 | ready，目标已确认，批次有效，入队权限有效 | queued |
| queued | 无阻塞且获得目标执行权 | validating；建立绑定具体目标 SHA 的 attempt |
| validating | 检查通过且执行前所有版本仍匹配 | integrating；先持久化执行意图，再派发 |
| validating | 检查失败或出现冲突 | queued + 具体阻塞原因；不定时反复运行同一失败候选 |
| queued/validating | 目标 HEAD 正常推进 | queued；废弃旧 attempt，保留入队序号，针对新目标验证 |
| queued/validating | 源版本变化（非固定 SHA）、目标 ref 或计划变化 | superseded；新批次条件满足后重新排队 |
| queued/validating | 人工取消 | cancelled；无后续 Git 写入 |
| queued/validating | 暂停自动化或撤销执行权 | queued + authority_paused；当前验证结果可记录，不能执行 |
| integrating | 从权威目标确认本次成功 | confirmed；保存实际结果，更新依赖 |
| integrating | 结果不明确、超时或竞争状态无法确定 | reconciliation_required；冻结该目标下一次执行 |
| integrating | 后端明确拒绝且证明未执行 | queued + 原因；取消已请求则 cancelled，批次已失效则 superseded |
| reconciliation_required | 查明已执行 | confirmed；即使后来又被回退，另记 withdrawn |
| reconciliation_required | 查明未执行且旧请求不能再生效 | 按当前取消/版本/权限进入 cancelled、superseded 或 queued |
| reconciliation_required | 仍无充分证据 | 保持原状；展示待对账 |
| queued/validating | 明确证明不需要新的集成 | no_change；有采用记录则关联，没有则只称无需变更 |

confirmed、cancelled、superseded、no_change 是不可逆终态。重新交付创建新 entry_id。未列出的状态转换拒绝并记录 validation_error，绝不隐式执行 Git 写入。

“检查失败”通过源/目标/配置/依赖变化或明确重试请求解除；仅时间经过不解除失败状态。临时读取失败允许有界退避重试，最多三次后显示连接阻塞，刷新或连接恢复后重新读取。

## 5. 依赖、队列公平性和影响范围

依赖最小字段：dependency_id、producer_route、delivery_id 或明确成果版本、scope、required_target、predicate、evidence_policy。

默认 predicate 是“指定成果在 required_target 当前版本有效”，不是“A 曾经合并过”。只要求历史完成事实的依赖必须显式声明，不能作为当前代码兼容性证明。依赖不得使用含糊的“始终跟随最新版”。

- A 撤回后：使需要 A 当前成果的 B/C 失效。未执行候选加入 dependency_withdrawn；已集成的 B/C 只报警并重评估，不自动回退。
- A 部分 cherry-pick：只满足有可靠映射和验收证据的成果范围；其余继续阻塞。无法映射时用 dependency_unknown。
- A 成果在 dev 有效、不在 main：只满足声明 dev 的依赖。
- A 只是改写提交而成果可能保留：进入 unknown 并重验，不能直接当成撤回或保留。
- 依赖环阻塞环内成员及实际依赖它们的候选，不阻塞无关路线。

队列采用“显式优先级，同级按入队序号”，跳过阻塞候选。改变目标 SHA 不改变排队位置；更换源快照/目标 ref/计划会产生新入队序号。已授权人工调序留下记录，不能中断正在执行的原子动作。

持续高优先级工作可能使低优先级等待，不自动提升优先级。每被十个后来入队且无依赖的候选超越一次，显示等待提示并允许有权限的用户调序；提示不是回撤警报。

## 6. 取消、暂停、放弃、删除及竞态

| 用户动作 | 默认效果 | 不附带的动作 |
| --- | --- | --- |
| 暂停自动化 | 禁止新的自动写入与自动派发；继续观测和对账 | 不取消路线、不删除代码 |
| 取消排队 | 取消特定批次；开发可继续 | 不删除 branch/worktree |
| 放弃路线 | 标记 abandoned，取消待执行批次，折叠计划 | 不撤回已经合入的内容 |
| 删除工作区 | 独立、明确授权的清理操作，遵循既有安全检查 | 不删除路线历史，不推定 branch 也应删除 |

每目标的协调日志串行记录取消、权限变更与派发请求，使用递增版本而非客户端时间排序。派发前比较最新权限版本、批次版本和目标 SHA。

取消先于派发边界：不得提交执行请求。派发已经发生：标记 cancel_requested，要求后端取消；只有后端证明未执行且不会迟到执行才标记 cancelled。已执行就记录 confirmed + “取消到达时已执行”，不以自动回退模拟取消成功。

人工回退与自动合并同时发生：如尚未派发则旧候选失效；如已经派发，尽力取消并查询后端事实。最终保存权威目标的实际事件顺序，检测是否重新引入撤回内容并报警。不能承诺取消一定能阻止已接受的远端动作，也不能用事后自动写入弥补。

## 7. 执行所有权、崩溃与幂等

每个目标只有一个已配置执行后端：provider 原生队列，或具有等价保证的协调器。启用自动执行前必须验证：串行更新、绑定候选版本、条件目标更新、可查询结果，以及取消/失效候选不会迟到执行的能力。不能满足时只提供观测和入队建议。

自建协调器必须使用持久化递增 epoch/fencing token；接收写入的端点拒绝旧 token。租约超时并不证明旧执行者停止；不能只凭本地锁过期派发新的写入。旧 worker 恢复后只能上报观测，不能沿用旧执行权。

对未采用固定 SHA 的源分支交付，后端必须在实际执行时校验源版本，或保证被撤销候选不会执行。只能检查 target SHA 而无法控制过期源候选的后端，不支持该模式自动集成。

attempt 保存 action_id、entry_id、source_sha、expected_target_sha、strategy、candidate_id、plan/policy revisions、executor_epoch、远端请求标识、预期结果及事件日志。幂等键属于同一次尝试；新目标候选使用新 attempt，不能复用旧验证结果。

| 中断位置 | 恢复方式 |
| --- | --- |
| 执行意图尚未持久化 | 没有可执行请求；重新读取状态后规划 |
| 已记录意图，尚不能确认是否派发 | 按 action_id 查询后端；当作不确定，不直接重发 |
| 已派发，未收到响应 | 查询操作结果和目标历史；有明确幂等保证时才能用同键重试 |
| 已成功，未写本地结果 | 依据后端与目标证据补记一次 confirmed |
| 曾成功，随后人工回退 | 补记 confirmed，再补记撤回；当前 HEAD 回到旧值不能证明“从未执行” |
| 已确认 Git，Context 写入失败 | 只重试幂等 Context 发布，不重复集成 |
| 目标不可访问、旧请求状态未知 | 保持 reconciliation_required，冻结该目标执行，其他目标继续 |

当前 HEAD 与执行前相同不足以证明没执行过，因为可能发生合并后又 reset。provider 记录、可用审计与目标观测冲突时保留证据并请求对账。

## 8. Cherry-pick、回退与异常回撤

### 8.1 Cherry-pick

Cherry-pick 应用选定提交的变化，通常产生新的提交；不能由此推定整条源 branch 被合并。[Git cherry-pick 文档](https://git-scm.com/docs/git-cherry-pick)

- 完整采用某个明确成果：记录 original/result SHA、范围和验证；仅对该成果更新效力。
- 部分采用：显示“部分采用 2/3 项”仅限有三个明确成果单元时；不能按提交数量估算需求完成率。
- 手工修改后采用、冲突解决后采用：映射待核实，不能只靠标题或 patch 相似度标为完整。
- 剩余候选默认失效并重新评估差异；不得自动把未选择的部分补进去。新的交付范围需要明确计划/人工确认，不能沿用旧的“完整合入”授权覆盖此次选择。
- 人工 cherry-pick 恢复已撤回内容：只有有明确恢复意图与范围时，替代对应保护记录；否则记录事实、提示“可能重新引入撤回内容”，不反向撤销人工操作。

### 8.2 Revert、reset 和手工回撤

Revert 创建新的提交来撤销先前变化，原提交仍在历史中；reset 的分支形式可以移动 HEAD，工作区/index 的变化取决于模式。两者的地图含义不同。[Git revert 文档](https://git-scm.com/docs/git-revert)、[Git reset 文档](https://git-scm.com/docs/git-reset)

- Revert：沿真实父关系增加新实心站点；原采用站点保留，详情关联“撤回了哪些成果”。撤回不是向旧站点画一条假的父边。
- Reset/force-push：移动已观测引用；原历史标记“不再被此分支引用”，已保存证据不删除。旧对象无法读取时显示证据缺失，不声称能恢复完整代码。
- 手工删除/修改：只有明确范围或验证证据时确认撤回；否则仅记录疑似效力变化。
- 正在进行的 cherry-pick/revert/rebase/merge 冲突：该工作区自动写入暂停，不自动 continue/abort，不触碰人工暂存区；与该工作区无关的安全任务可继续。
- Revert 一个 merge 后，原提交可能仍可达，后续 merge 不保证恢复撤回内容；本系统不得靠重复 merge 来“修复”。[Git revert 文档](https://git-scm.com/docs/git-revert)

### 8.3 防止自动恢复的记录

建立 WithdrawalGuard：guard_id、目标仓库/ref 或本地工作区范围、原采用/成果范围、变更前后 SHA、人工意图依据或 unknown、状态 active/superseded、replacement_instruction_ref。

人工撤回明确范围后，所有可能恢复该范围的自动候选加入 human_withdrawal_guard。更换分支名、source SHA、route_id 或新开会话不绕过该记录。Guard 不因时间经过或用户仅点击“已知晓”自动解除。

只通过新的明确恢复指令或可靠的范围澄清替代 Guard。恢复指令也必须重新走当前检查；测试通过本身不构成恢复授权。

范围无法确定时，对发生变化的目标暂时阻塞自动集成并显示 scope_unknown；一旦能界定范围，缩小到受影响候选。无法可靠证明候选无关时保持阻塞，不假装能判断任意代码的语义等价。

## 9. 警报判据与地图表现

“不合理”必须落到事实或声明的约束，AI 推测不能单独构成确定性严重警报。即使是有意的人工回退，系统仍报告对其他工作造成的影响。

| 级别 / 代码 | 触发条件 | 自动化效果 |
| --- | --- | --- |
| 信息 human_change_recorded | 已确认人工操作，范围清楚，没有发现额外约束受损 | 保留结果；撤回保护仍对恢复候选有效 |
| 黄色 rollback_suspected | 内容与已采用成果可能相反，但缺乏可靠映射 | 相关候选待核实，不称“错误回退” |
| 黄色 history_rewritten | 非快进目标变化，尚未判明成果损失 | 旧候选失效、目标先对账 |
| 黄色 actor_or_scope_unknown | 操作者或作用范围不明且涉及待执行工作 | 保留事实，阻塞受影响自动写入 |
| 红色 dependency_withdrawn | 明确撤回当前候选依赖的成果 | 阻塞依赖闭包内候选；已合入依赖方仅报警 |
| 红色 required_check_regressed | 当前版本配置的必需检查失败 | 阻塞相关集成；只有额外因果证据才称由回退导致 |
| 红色 withdrawn_content_reintroduced | 有可靠映射证明未经新的恢复指令重新引入撤回范围 | 阻止尚未执行的候选；若已发生则报警，不自动反向修改 |
| 红色 execution_inconsistent | 后端结果、动作日志与权威目标无法一致解释 | 冻结该目标执行并对账 |

每条警报保存 alert_id、rule_id、scope、before/after SHA、证据、受影响路线、首次/最近观测、状态。相同规则+范围+变化事件去重；新事件即使 SHA 相同也可形成新警报，避免 reset 到同一 SHA 时漏报。

警报生命周期：open → acknowledged → resolved。acknowledged 仅表示已知晓，不解除 gate 或 Guard。resolved 必须有原因与证据（重新验证通过、范围澄清、已批准计划改变等）；接受当前回退可解决意图疑问，但不把仍然失败的检查变成通过。

地图：

- 已发生的操作始终保持实线；用黄色/红色图标和短标签表达警报。
- 原成果显示“曾合入 · 已人工撤回”；相关未来路线仍是虚线，显示“等待：依赖已撤回”。
- Cherry-pick 用“部分采用 / 已采用”标签和详情映射，不伪造 merge 轨道。
- 选中警报展示变更前后、受影响成果与任务、依据和下一步动作。
- 提供“查看差异”“确认操作意图”“调整计划”“请求恢复”；不提供默认自动恢复按钮。
- 警报折叠时仍在所属路线与目标上显示计数；不只依赖颜色。断网时显示观测时间，不把陈旧快照当成新警报。

本轮只设计地图内警报，不新增邮件、消息或后台通知。

## 10. 事件处理表

所有事件先验证仓库/目标范围、去重并追加记录，再归约状态。跨宿主不依赖墙上时钟排序；外部事件触发重新查询权威状态，不能让迟到事件把 observed_head 回拨。历史事件可补入审计链。

| 事件 | 状态影响 | 记录及地图 |
| --- | --- | --- |
| session_rebound | 路线保持，执行者关联更新 | 不增加新路线/假 commit |
| source_changed | 普通旧批次 superseded；固定 SHA 例外按第 3 节 | 当前位置更新，旧计划证据保留 |
| target_advanced | 未执行 attempt 失效，队列位置不变 | 计划接入位置更新 |
| target_rewritten | 对账并重评估成果，范围不明时目标阻塞 | 新事实+历史改写警报 |
| target_missing | target_missing 阻塞，不改投其他分支 | 目的地缺失，不连到相似站点 |
| plan_changed | 新 revision，旧条目 superseded | 旧计划归档、新计划虚线 |
| permission_revoked | 未派发禁止执行；已派发按竞态处理 | 等待授权或结果待核对 |
| manual_pick_confirmed | 采用范围更新，旧候选重新评估 | 部分/完整采用及来源 |
| manual_withdrawal_confirmed | Guard 生效，依赖重评估 | 已撤回标签和影响警报 |
| restoration_requested | 指定 Guard 可被有权限指令替代，建立新候选 | 新恢复计划；事实层不预先改变 |
| external_integration_observed | 确认范围充分则 confirmed/no_change；否则对账 | 不重复合并、不捏造完整采用 |
| check_result_received | 只用于匹配 SHA/配置/计划的候选 | 迟到结果保留但标 obsolete |
| backend_timeout | reconciliation_required | 下一项不抢先执行 |
| context_publish_failed | Git 结果保持 confirmed | Context 待发布警报 |
| duplicate_event | 无重复状态转换、无重复动作 | 引用已有记录 |

## 11. 验收场景表

以下是未来自动化测试与 UI 检查的输入/输出契约，不表示本轮运行过测试。所有场景均检查 action journal 与没有额外 Git 写入。

| ID | 初始条件与事件 | 必须得到的结果 |
| --- | --- | --- |
| B01 | 代码检查通过，必需人工验收缺失 | Route 保持 active，不入队 |
| B02 | 未定义完成条件 | 不自动 ready，显示缺失条件 |
| B03 | 普通批次排队后 source 新增提交 | 旧条目 superseded；新版本重新验收，不能替换旧 SHA 执行 |
| B04 | 明确固定 SHA 批次，branch 继续开发 | 批次仍绑定原 SHA；新工作另显示，不能标全部完成 |
| B05 | A/B 同目标排队，A 合入 | B 原验证失效，保留序号并针对新目标重验 |
| B06 | A 阻塞，B 无依赖 | B 可先行，A 保留序号 |
| B07 | B 依赖 A 当前成果，A 人工 revert | A 历史仍 confirmed、成果 withdrawn；B 阻塞；没有自动恢复命令 |
| B08 | B 已合入后 A 撤回 | B 报依赖风险并重验，不自动回退 B |
| B09 | A 三项明确成果，人工只 pick 两项 | 只记录两项采用，第三项不自动补入，不标整路线 merged |
| B10 | pick 经人工修改，不能可靠映射 | partial/unknown 按证据显示，相关依赖不假装满足 |
| B11 | 已确认人工回退且无额外失败 | 信息标签+Guard；不指责人工操作异常、不恢复 |
| B12 | 已确认人工回退，必需检查失败 | 保留人工基线+红色检查警报；不自动 revert-of-revert |
| B13 | 提交标题写 Revert、作者是人名，无可信操作证据 | 不推定人工来源；显示外部变化，按实际影响核对 |
| B14 | 本地 reset，远端未变 | 仅本地基线/工作区阻塞，远端集成事实保持原样 |
| B15 | 远端非快进且范围不明 | 目标自动集成暂停，显示 history_rewritten，其他目标可继续 |
| B16 | 只剩旧 SHA 可达性，但成果已经 revert | 不因可达性将成果标 present |
| B17 | 新路线改名后试图恢复撤回范围 | Guard 仍命中，不绕过保护 |
| B18 | 用户点击警报已知晓 | acknowledged；Guard 与失败检查仍生效 |
| B19 | 人工明确恢复 Guard G，随后检查通过 | 新候选可执行；保留回退与恢复记录，不修改旧历史 |
| B20 | 取消先于派发边界 | cancelled，后端没有写请求 |
| B21 | 已派发后取消，后端随后成功 | confirmed + 取消迟到记录，不自动回退 |
| B22 | 回退与合并同时发生且重新引入被撤回成果 | 保存实际顺序，可靠命中则红色警报；无自动反向写入 |
| B23 | 成功后崩溃，本地没有结果记录 | 查询后补一次 confirmed，不重复集成 |
| B24 | 成功后人工 reset 回旧 HEAD，再重启 | 不因 HEAD 相同判定未执行；补历史或保持对账 |
| B25 | 旧 worker 租约到期后恢复 | 后端拒绝旧 epoch 写入；单一有效执行者 |
| B26 | 后端不能绑定有效候选或查询结果 | 不开放自动集成能力，只展示建议 |
| B27 | Git 确认成功、Context 发布失败 | 只重试 Context；图显示分别的状态 |
| B28 | 重复/乱序 webhook、旧测试结果到达 | 不重复动作，HEAD 不倒退，旧证据不用于新候选 |
| B29 | A 成果只进入 dev，B 要求 main | B 依赖仍未满足 |
| B30 | A/B 依赖成环，C 无关 | A/B 阻塞，C 可执行 |
| B31 | 人工 cherry-pick 正处于冲突且 index 有修改 | 自动化不 continue/abort/stage；相关工作区暂停 |
| B32 | 放弃路线或取消排队 | 不删除工作区、不撤回已集成内容 |
| B33 | 目标被删除、出现同名其他远端分支 | 不自动重新绑定；目的地缺失 |
| B34 | 原目标成果没有变化、没有新交付差异 | no_change；无采用证据不称发生过 merge |
| B35 | 重复观察同一回退事件 | 同一警报更新观测；不重复新增 |
| B36 | 新回退事件恰好回到此前相同 SHA | 保存新事件并重新检查，不被旧去重规则吞掉 |
| B37 | 已记录历史对象后来不可读 | 地图显示证据缺失；不编造父边或声称可恢复代码 |
| B38 | 计划变更、暂停、放弃分别发生 | 按对应动作处理，三种行为不混淆 |

## 12. 实施交接与范围限制

实施前将本文件规则映射到独立任务：状态归约器、候选/队列、执行后端能力与恢复、成果/撤回记录、地图警报。每项使用 B01–B38 中相关场景作为验收输入，另保留原设计的安全与归因测试。

第一版不承诺识别所有语义回归或任意代码等价。能证明则标记，不能证明则 unknown 并阻塞相关自动动作；不通过推测代码动机解决不确定性。

自动远程 merge 上线还要求选定实际 provider 并验证第 7 节能力；本设计没有声称任意 provider 已满足。文档设计完成不等于实现或端到端验证完成。
