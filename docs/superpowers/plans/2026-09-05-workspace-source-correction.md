# 地图来源与执行位置：根因与修复

## 复现与根因

预览从项目根目录启动，根目录 HEAD 为 `50bb4833`；实际代码修改在 `devmap-journey-focus` Worktree，其 HEAD 为 `fef7a78`。预览脚本使用 `out.parents[3]` 回到了旧根目录。

后端 `current_worktree_id` 来自扫描器对 MCP `SourceWorkspace.root` 的匹配，它表示地图来源。宿主任务的登记 cwd 是另一种关联信息；两者都不能证明 Agent 后续每条命令的实际 cwd。

界面把来源字段标成 `This task workspace`，而测试也把这个说法当成正确结果。因此这是启动路径错误加上产品语义与测试缺口，能够在任务根目录固定、Agent 切换 Worktree、旧 MCP 进程继续运行等场景复现。代码没有回退。

## 已修改

- 地图及列表统一显示 `Map source`；按钮为 `Locate source`，说明来源并非已验证的 Agent 执行位置。
- `view: agent` 增加 `workspace_selection`：`basis` 为 `map_source_default` 或 `explicit_entity`，同时返回 `map_source_worktree_id` 与 `execution_location_verified: false`。
- 显式选择实体仅证明读取对象选择，不当作实际执行目录证明；兼容字段 `current_worktree_id` 保留原始来源语义。
- Skill 要求在 Git 操作前核对实际命令 cwd、Git worktree 根目录和 HEAD，再使用精确 worktree ID 读取。不得用项目根目录、最新提交、最近修改目录或 UI 选择替代这一步。
- 预览入口已改到实际开发 Worktree；后续预览从实际执行 cwd 解析 Git 根目录，并核对它与本构建工作区一致。

## 回归覆盖

- 旧地图来源与另一个选中工作区并存时，来源高亮不冒充执行位置。
- 不带 entity_id 的读取明确标注来源默认值；带 entity_id 也不伪造执行验证。
- 临时仓库根目录落后于开发 Worktree：默认读取仍返回根目录事实，显式读取返回开发目录事实，两个不同 HEAD 不混用。
- 保留原有任务关联、已提交/未提交状态、只读行为和资源预算测试。

## 仍然存在的边界

此修复消除了产品对执行位置的无依据断言，**没有增加能自动观察任意宿主命令 cwd 的能力**。受控执行层尚未实现；未来必须使用每次具体操作的明确 worktree ID、实际执行目录与预期 HEAD，不能仅信地图选中状态。旧安装版本需要更新后才具备这些修正。
