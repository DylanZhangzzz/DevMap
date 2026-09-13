# visible-workspace-chats 分支核对与详情区调整

核对对象：`codex/visible-workspace-chats` 的 `8d24a61af8e2cd43be4e866e041b6247cc457649`；当前简化分支基于 `ac24f14`。Git 的 `HEAD..codex/visible-workspace-chats` 仍列出该提交，因此不能称为已完整合并。以下逐项按现有代码比对，不用“界面相似”代替功能覆盖。

| 原分支修改 | 当前覆盖情况 |
|---|---|
| 每工作区独立聊天卡、卡片高度和轨道避让 | `assets/metro-core.js` 与该分支完全一致；HTML 聊天卡主体已存在 |
| 窄屏聊天标题与 Current chat 标签对齐 | 原 grid 修复遗漏；本次恢复 |
| 横竖方向切换后重新定位选中工作区 | 原方向变化处理遗漏；本次恢复 |
| 刷新时的选中、焦点和滚动 | 已有；当前还包含 1cc1831 的先插入后移除及选中标记恢复，本次保留 |
| 196 KiB 资源预算 | 已有对应预算注释和实现；资源大小约束仍需随产物检查 |
| 工作目录报告独立保存、跨 Viewer/重启恢复、延迟报告与等时冲突 | **未完整覆盖**。原分支通过 journal 独立报告文件和 Dock 刷新加载实现；当前 `application.rs` 只合并该 ClientView 中保留的报告。不能将客户端内保留等同跨客户端/重启持久化，等时冲突也未按原方案拒绝 |
| 对上述持久化的模型/损坏文件回归 | 原新增测试未完整移入当前架构；没有假称已通过 |
| Skill/MCP 的持久化保证文案与历史截图 | 文案/历史图不是功能证明；实际保证以本表限制为准 |

本次首先完成用户要求的覆盖核对，没有整包 cherry-pick：将旧 JSON 权威文件原样引回会影响仓库级 SQLite 设计。独立工作目录报告的迁移与持久化仍是明确功能缺口，不因本次 UI 修复而关闭。

## 本次界面修改

- 展开的卡片与下方详情使用同一背景、连续边框、对齐的内边距和相接的圆角。
- Branch、完整 HEAD、Worktree 原值旁提供 Copy；成功反馈 Copied，剪贴板拒绝时提示 Retry，并保留可选中的原值。
- 删除展开区的 View full details 按钮，将创建记录、计划起点、共同祖先、绑定记录、事实/风险与全部任务详情放进展开区，保留计划的专门查看操作。
- 保留其他对象的底部 inspector；不是删除提交、任务、路线的详情能力。

105 项渲染与交互检查通过，包含精确复制值和剪贴板失败分支。旧“两条预览后跳转”测试调整为全部任务直接可达；任务消失的焦点回退仍检查真正移除，而不是把归档误认为消失。新 release 构建成功；实际浏览器结果另存于 `target/verification/manual-details-preview`。用户人工验收政策不变。

实际浏览器首轮发现 420 px 视口中的长值被挤成单字列，已改为字段名在上、值与复制在下。最终 420/1000 px 两种视口无横向溢出，值列宽分别为 97/239 px，三个按钮均在真实浏览器中复制完整原值。最终 HTML 为 199253 字节，低于原 196 KiB 预算。确认截图与 browser-check.json 位于 target/verification/manual-details-final；首轮结果保留在 manual-details-preview。未追加整批性能验收。

最终构件：target/verification/manual-details-final/devmap.exe，SHA-256 786657AD28900CB63F7B14DBA7DED7A057E8C88CB4A826920662C3C3B34F9BBB。它包含当前未提交时构建的 UI 源码，Build 标签可能显示 ac24f149-dirty；请以构件哈希识别。新 MCP Viewer 已交给右侧 Browser 打开，工具返回 queued，未声称 UI 已前台显示；原 guide 页面保留。
