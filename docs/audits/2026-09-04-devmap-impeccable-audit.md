# DevMap — Impeccable 开发前审查

日期：2026-09-04。范围：当前本地 Browser Dock、对应 HTML/CSS/JavaScript、Git 关系与任务清单模型，以及已通过的相关测试。

## Implementation Integrity Verdict — 未通过交付验收

当前实现具备真实 Workspace/任务身份和基本操作能力，但还不是用户选定的完整 Metro 拓扑。它按 Worktree 的目标分支与共同祖先组织显示，并未提供所有 branch refs 的 commit DAG；浏览器实测还确认站点圆心与主线相差 **21 CSS px**。因此，下方分支关系不清楚不能仅归因于设计图的绘制精度。

保留已认可的浅色风格和图 3 的地铁图语言；不得照搬图 3 中缺失的来源连接、悬空的 merge 箭头或以警告色替换分支身份色的错误。

这是一轮 **Impeccable audit 技术审查**，不是已完成的修复、完整 WCAG 认证或安装验证。未修改生产代码，未调用任何 Git 写操作或安装操作。

## Executive Summary

- Audit Health Score：**9/20 — Poor / 需要结构性修正**。这是技术质量分数，不是否定已获认可的配色。
- 已确认问题：**P0 0 项，P1 7 项，P2 2 项，P3 0 项**。
- 最高优先级：真实拓扑数据、连线坐标、窄侧栏首屏、状态事实的区分。
- 当前 57 项相关测试通过，但没有覆盖实测线条偏移和完整 branch 拓扑。
- 下一步按[开发计划](../superpowers/plans/2026-09-04-devmap-metro-topology.md)补充数据契约与回归测试，再实现图层及交互。

| # | Dimension | Score / 4 | Key finding |
|---|---|---:|---|
| 1 | Accessibility | 2 | 有按钮语义、键盘操作与焦点处理；功能字号过小，部分文本对比度不足 |
| 2 | Performance | 2 | 自包含资源、有字节预算；内容刷新重建节点，扩展图数据后的性能尚未验证 |
| 3 | Responsive Design | 1 | 窄侧栏默认只露出部分当前 Workspace，内边距响应式规则未生效 |
| 4 | Theming | 3 | 浅色 tokens 与产品方向一致；部分状态色/硬编码样式仍需整理 |
| 5 | Implementation Integrity | 1 | 缺真实 branch DAG；站点不在线上；integration 与 working-state 表达易误导 |
| **Total** | | **9/20** | **Poor — major overhaul of topology/operational clarity** |

Performance 为源码与当前有限数据量下的暂定分，未执行新增大图的帧率或长会话性能测试。不因用户未要求暗色模式而扣分。

## 证据与来源

### 审查对象与版本

- 源码工作区：`C:/Users/user/Documents/ChatGPT/DevMap-phase-1a-worktree`
- 分支：`codex/rail-view-design-alignment`
- HEAD：`5741c330c457331065d76dc884ff5b0ea8a2c2f0`，另有审查前已存在的未提交修改。
- 当前页面：本机端口 **50576**；鉴权 token 不记录。
- 页面文档标题：`DevMap Git Work Map`。
- 可见页眉：`DevMap · Rail View`。
- 可见主标题：`Repository topology`。
- 监听进程：PID **35164**。
- 实际可执行文件：`C:/Users/user/AppData/Local/Temp/devmap-preview-worktree-label/debug/devmap.exe`。
- 当前 `assets/dock.html` SHA-256：`623aa7a1770214e4a63dbd2c6271ba44de18e56fdf1f0c61d34a686fdc6b715a`。
- 已将浏览器实际 style/script 与本地文件对应内容逐项比较（统一换行、去首尾空白）：**两者一致**。

这是临时 debug 预览，不证明已安装 skill/MCP 运行的是同一构建。交付时必须重新核验版本化进程路径和实际资源指纹，不能沿用历史版本号作为当次预期。

### 浏览器截图

视觉对照：[用户选定的图 3（原始概念图，非已实现页面）](assets/2026-09-04-metro-preflight/approved-concept-3.png)。图中缺失连接等问题以本报告及开发规格的修正规则为准。

本轮批量采集后已恢复原视口并关闭临时审查标签页；保留用户原标签和运行服务。

- [原侧栏](assets/2026-09-04-metro-preflight/current-sidebar.png)
- [360px 侧栏](assets/2026-09-04-metro-preflight/sidebar-360.png)
- [480px 侧栏](assets/2026-09-04-metro-preflight/sidebar-480.png)
- [900px 展开窗口](assets/2026-09-04-metro-preflight/sidebar-900.png)
- [READ 密度与向右平移](assets/2026-09-04-metro-preflight/read-density-end.png)

实际页面在此批次显示 7 个 Workspaces、2 个 Linked chats、1 个 Not merged。当前采集到的两个任务名是“查找今天上午开发结果 (2)”和“调研 worktree 下拉菜单接口”；这不证明完整宿主清单只有两个任务。

截图中的 dirty 文件数会随新增审查文档增加，不作为代码错误的固定数量断言。

| 视口宽度 | shell 水平 padding | 画布宽度 | station 圆心偏离 rail 中线 |
|---|---:|---:|---:|
| 360px | 24px | 2760px | 21px |
| 480px | 24px | 2760px | 21px |
| 900px | 24px | 2760px | 21px |

这些数字来自实际 DOM 几何与 computed style，不是对截图肉眼估算。画布内横向滚动本身符合用户要求，问题是默认取景与操作内容被挤出首屏。

## Detailed Findings

### [P1-01] 当前画出的不是完整 Branch/Commit 拓扑

- **Location:** `src/git_relationship.rs:255` 起的 integration branch 选择、`src/git_relationship.rs:326` 的关系计算；`assets/dock.html:336` 的 createRail。
- **Category:** Implementation Integrity。
- **Evidence:** 数据聚焦 integration target、共同祖先、ahead/behind 与 Worktree；renderer 为 integration branches 创建轨道，再摆放 Worktree stations，没有独立完整的 branch refs 与 commit parent edges。
- **Impact:** 非目标分支、feature-of-feature、无 Workspace 的分支等无法可靠显示。下层支线即使涂上地铁色，仍无法判断从哪里分出。
- **Standard:** 本项目已确认的 branch DAG 与 Workspace HEAD 归属契约，不涉及单独 WCAG 条款。
- **Recommendation:** 先提供有界的真实 commit/ref 图和显式截断边界，再统一布局；禁止将 merge-base 自动称为历史 branch 创建事件。
- **Suggested command:** `$impeccable shape`，实现后 `$impeccable harden`；对应计划 Task 1–4。

### [P1-02] 站点与轨道没有共用连接坐标

- **Location:** `assets/dock.html:67`–72 的 rail/station 几何。
- **Category:** Implementation Integrity / Responsive Design。
- **Evidence:** 三种宽度均测得 44px station 外框与 2px rail 顶边对齐，而两者圆心/中线差 21px；900px 截图可见站点浮在主线下方。
- **Impact:** 用户无法分辨节点是否属于该轨道；拓扑的核心“连接”语言失效。
- **Standard:** 产品几何验收：连接端点误差不超过 1 CSS px。
- **Recommendation:** SVG 连线与语义节点共享 world coordinates；对 fork、merge、Workspace attachment 和离屏端点执行真实坐标断言。
- **Suggested command:** `$impeccable layout`；对应 Task 4–5。

### [P1-03] 窄侧栏首屏不能完整识别当前 Workspace

- **Location:** `assets/dock.html:23` 的 shell、`assets/dock.html:130` 附近 container query，以及 `assets/dock.html:315` 的 topologyWidth。
- **Category:** Responsive Design。
- **Evidence:** 360px / 480px 初始视图只露出当前 Workspace 的部分内容；所有测量宽度 shell 仍为 24px padding。规则试图在自身建立的 container query 内修改同一 .shell，未产生预期的 12px 结果。
- **Impact:** 最重要的“谁在当前哪里工作”需要先寻找和横向滚动，阅读成本高。
- **Standard:** 已确认的 360–526px 初始取景要求；二维图允许平移，不将图内横向滚动本身判为 WCAG reflow 违规。
- **Recommendation:** 使用能生效的父容器/媒体查询，压缩 main 标签占位；启动定位当前 Workspace，并提供可见的重新定位入口，保持可读字号而非整体缩小。
- **Suggested command:** `$impeccable adapt`；对应 Task 5。

### [P1-04] “Merged” 将 ancestry inclusion 表达成了完成的 merge

- **Location:** `src/git_relationship.rs:425`，`assets/dock.html:443` 的 createReturnState。
- **Category:** Implementation Integrity。
- **Evidence:** `merged: Some(ahead == 0)` 只证明目标包含当前已有提交；页面同一 Workspace 同时显示 DIRTY 和绿色 “Merged → main”，没有明确“已有提交已包含，未提交修改仍存在”。
- **Impact:** 监督者容易把“提交已包含”理解为该工作已完全归并并安全收尾，遗漏本次最重要的未提交危险区。
- **Standard:** 产品状态事实契约。
- **Recommendation:** 独立呈现工作区修改、目标包含与已验证 merge commit；仅在真实事件有证据时画归并箭头。included + dirty 必须同时明确显示。
- **Suggested command:** `$impeccable clarify`；对应 Task 2–3、5。

### [P1-05] Git 读取失败的 fallback 会携带 false/0 而非未知

- **Location:** `src/git_relationship.rs:155` 错误路径，`src/git_relationship.rs:444` 的 unknown_relationship；`assets/dock.html:438` 的 working-state 呈现。
- **Category:** Implementation Integrity。
- **Evidence:** fallback 将 dirty 设为 false、changed_file_count 设为 0，字段没有表达 working state unknown。UI 的 clean/dirty 二分可据此显示 CLEAN。
- **Impact:** 无法读取的 Workspace 可能显得比实际更安全。此项为源码确定的错误路径风险；本轮没有在用户仓库中人为制造权限错误。
- **Standard:** 产品事实/未知状态契约。
- **Recommendation:** working_state 使用 clean/dirty/unknown；错误必须附着到对应 Workspace，保留最后已知值时明确标注时间与 stale。
- **Suggested command:** `$impeccable harden`；对应 Task 2–3。

### [P1-06] Git heartbeat 与任务清单新鲜度没有清晰区分

- **Location:** `src/dock.rs:466`–487 的 replace_observed_tasks；`assets/dock.html:261` 起的 snapshot 接收与 `assets/dock.html:654` 的 setOnline。
- **Category:** Implementation Integrity。
- **Evidence:** 相同任务清单的成功重采样不会更新 task_inventory_synced_at；Git 快照仍可持续显示 “LIVE · updated now”。MAP 默认不展示任务活动时间；READ 可看到约 5h ago。
- **Impact:** 用户可能把 Git 更新理解为 Agent 状态刚刚验证。缺少清单完整性和采样时效时，不能可靠判断“无人处理的修改”。
- **Standard:** 产品观察新鲜度契约。
- **Recommendation:** 分离 Git sampled_at、task inventory sampled_at、task last_activity；每次成功重采样更新时间但不必重排图。缺失/陈旧数据明确 UNKNOWN，不据此断言无人活跃。
- **Suggested command:** `$impeccable harden` / `$impeccable clarify`；对应 Task 2–3、6。
- **Caveat:** 5h 的最后活动时间本身不证明 active 状态为假；本轮没有完整宿主 inventory 新鲜度证据。

### [P1-07] 功能字号过小，部分文本状态色对比度不足

- **Location:** `assets/dock.html:8`–17 的 tokens，`assets/dock.html:76` 的 fork 标签，`assets/dock.html:88`、97–100、112 的 Workspace/任务/状态。
- **Category:** Accessibility / Theming。
- **Evidence:** 实际样式中 fork 8px、Workspace 10px、任务标题 11px、Agent/状态 9px。检测器报告 muted 对背景约 4.3:1、绿色文本约 2.5:1、蓝色 hover 文本约 3.7:1；使用场景与样式已核对。
- **Impact:** 用户必须放大或费力阅读任务和风险；颜色越淡、字号越小，侧栏越难快速扫读。
- **Standard:** 适用文本对比度基准为 WCAG 1.4.3 的 4.5:1；WCAG 没有绝对字号下限，14px/12px 是本项目可读性验收标准，不宣称其为 WCAG 规定。
- **Recommendation:** 主操作名称/任务标题至少 14px，次级功能文字至少 12px；分支线色与正文色分开，黄线增加对比边界。不能为适应侧栏而缩小整张图。
- **Suggested command:** `$impeccable typeset` / `$impeccable colorize`；对应 Task 5。

### [P2-08] 内容更新重建节点，选择态与详情缺乏统一恢复

- **Location:** `assets/dock.html:332` 的 surface.replaceChildren、`assets/dock.html:581` 的 aria-current 选择、`assets/dock.html:613` 的 showDetails。
- **Category:** Performance / Implementation Integrity。
- **Evidence:** 源码在内容渲染时替换节点，选择仅附着在当时 DOM 上；详情通过一次点击填充，没有完整的稳定 selected ID 驱动刷新链路。展开状态已有独立集合，是可复用基础。
- **Impact:** 有内容变更时可能丢失焦点/选中边框，详情可能仍指向旧数据。此项为源码路径审查，尚未注入更新 fixture 做浏览器回放。
- **Standard:** 产品 live-update 行为契约。
- **Recommendation:** 按稳定身份保存 selection/expansion，刷新选择详情，必要时 keyed patch；图布局仅随结构变化更新。
- **Suggested command:** `$impeccable harden` / `$impeccable optimize`；对应 Task 6。

### [P2-09] 操作反馈与连接状态复用同一输出区

- **Location:** `assets/dock.html:626` 的 announce 与 `assets/dock.html:654` 的 setOnline。
- **Category:** Implementation Integrity / Accessibility。
- **Evidence:** 导航/密度等反馈写入 #connection，下一次 online heartbeat 又写成 “LIVE · updated now”。
- **Impact:** 用户可能来不及读到导航失败、无可验证任务链接等说明；不能靠短暂变色判定跳转成功。
- **Standard:** 操作反馈需可感知且持续足够时间；本轮不据此直接宣称某个完整 WCAG 条款已失败。
- **Recommendation:** 独立连接状态、采样状态与 aria-live 操作反馈；pending/error 保留至有结果或用户清除。
- **Suggested command:** `$impeccable harden`；对应 Task 6。

## Detector 结果解释

运行一次 `impeccable.cmd detect --json assets/dock.html`，共 **13 条 warning**，不是 13 个独立已证实缺陷：

- 3 条 low-contrast：纳入 P1-07；比例基于检测器的具体颜色组合，实际背景变化仍须逐状态复测。
- 2 条 tiny-text 与 6 条 undersized-ui-text：合并为 P1-07；computed style 还确认 8–9px 动态标签。
- 1 条 overused-font（Inter）：**不作为缺陷**。这是高频操作型工具，熟悉且清晰的字体符合用途；不为“差异化”强行换字。
- 1 条 nested-cards：**条件性采纳**。Workspace 包含任务是用户确认的真实层级，不应一律拆除容器。只削减多余外框与占位，纳入布局工作，不单独计数。
- 检测器行号为 0，报告中的定位来自源码复核，未把行号 0 当成有效定位。

原始结果见 [impeccable-detect.json](assets/2026-09-04-metro-preflight/impeccable-detect.json)。

## 已验证交互、测试与尚未验证项

已验证：

- Workspace 点击后可显示对应稳定路径、分支、HEAD 等详情。
- READ / MAP 密度切换生效。
- 键盘 End 向右平移，Home 回到起点；存在可用横向滚动。
- 使用真实任务标题和验证过的任务身份生成记录，没有从截断文字猜测 ID。
- `cargo test --test git_relationship --test dock_model --test dock_ui_contract`：**9 + 16 + 32 = 57 passed，0 failed**。

尚未验证，保留为交付 gate 而非臆断缺陷：

- 点击真实任务后，宿主确实切到相应 task；本轮没有发送会导致任务导航的宿主消息。
- 全量本地 active/idle/history 清单的完整性与当前性。
- 新图的所有 synthetic DAG 场景、200% 文字缩放、1440px 布局、大图性能。
- 断网/恢复、读取失败、任务移动/改名、导航失败等端到端回放。
- 最新已安装 skill/MCP 与当前预览的一致性。

## Patterns & Positive Findings

系统性问题是“关系摘要替代图数据”“装饰布局替代共享几何”“不同事实复用一个状态”和“用小字号容纳更多内容”。这些需要统一数据/布局契约，不能只修单个 CSS offset。

应保留的基础：

- Workspace 主名称与 branch 元数据已分开，方向符合用户要求。
- 任务归属以实际路径/身份为依据，而非项目名模糊匹配。
- 按钮语义、键盘平移、拖动区域保护和任务导航超时已有基础实现。
- 自包含 HTML、模型/资源字节限制、文本转义和任务 ID 校验已有测试。
- 浅色 token 系统与用户认可的整体视觉风格，不需要推倒重选配色。

## Recommended Actions

顺序已反映在开发计划，不需要用户重复选择设计方案：

1. **[P1] `$impeccable shape` / `$impeccable harden`**：固定真实图数据、未知状态、新鲜度和稳定身份；已写出准备阶段的契约。
2. **[P1] `$impeccable layout` / `$impeccable adapt`**：统一 SVG/DOM 坐标，修正首屏取景和多宽度几何。
3. **[P1] `$impeccable clarify` / `$impeccable typeset` / `$impeccable colorize`**：分开状态事实，改善字号与对比度，保留地铁分支色。
4. **[P2] `$impeccable harden` / `$impeccable optimize`**：稳定选择、任务跳转反馈与刷新成本。
5. **`$impeccable audit`**：修复后用同一维度、同一真实侧栏与 synthetic fixtures 复查。
6. **`$impeccable polish`**：仅在图关系、状态和交互验收后收尾。

这些步骤可分项或批量执行；本轮不再要求选择顺序。修复后的 audit 和真实运行验证通过前，不将页面标为“开发完成”。
