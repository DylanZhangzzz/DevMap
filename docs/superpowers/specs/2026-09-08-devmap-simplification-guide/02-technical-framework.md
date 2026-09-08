# DevMap 简化技术框架

状态：建议方案，等待评审；不是现有系统实现说明。目标读者：产品负责人及后续开发、验证人员。

## 1. 目标与范围

用户进入任意一个 worktree，打开 DevMap，就应看到这个仓库的工作区、任务、真实 Git 状态和已记录路线。初始化、状态保存和刷新由程序处理，日常操作不要求用户创建独立 Context Repository 或理解 JSON 日志、manifest、租约文件。

本轮简化覆盖本机、单用户、一个 Git 仓库及其多个 worktree。多仓库可以分别运行相同核心；跨机器同步、云服务、自动 merge/revert、代码 AST 索引、通用图查询语言均不进入首轮范围。Git 写操作仍属于独立、明确授权的工作流。

成功标准是：用户步骤减少，重复扫描和写入路径减少，恢复逻辑更集中，领域语义不退化。数据库文件数量减少只是手段。

## 2. 当前代码基线

DevMap 基线为本地 main `db696768baf366b442adf6097e0c1f4cec3f7e08`。所有现状判断仅针对该提交；当前分支、其他工作区和已安装运行时可能不同。

| 当前模块 | 已观察到的职责/存储 | 简化中的处理 |
|---|---|---|
| src/git.rs、worktrees.rs | Git 检查、common dir、worktree 发现及身份计算 | 保留并复用 |
| src/git_relationship.rs、git_topology.rs | Git 关系与拓扑计算 | 保留语义，统一输入快照 |
| src/journal.rs | 每会话 events.ndjson、intent、index、锁；任务关联与水位文件 | 以事务、唯一键及持久水位替换多文件提交协议 |
| src/presence.rs | common dir 下 presence/v1 的每会话 JSON 与锁；状态来源和租约 | 当前状态进入数据库；保留来源、置信与过期语义 |
| src/route_plan.rs | route-plans.jsonl；request_id、expected_revision、start_commit | 保留幂等、版本并发检查及不可变起点 |
| src/dock.rs | DockReadModel / DockReducer，schema 为 devmap/dock/4 | 先保持输出兼容，再扩展 freshness 字段 |
| src/viewer.rs、mcp.rs | 浏览器服务与 MCP 入口；已有本地访问 token | 适配共享应用服务，保留已有访问边界 |
| src/context.rs | 独立 Context Git 及对象完整性 | 作为兼容能力保留，退出默认初始化必需路径 |
| src/adapter.rs、hook.rs、capture.rs | 宿主接入、事件与能力边界 | 继续做薄适配，不能通过换库夸大观察覆盖率 |

源码入口：[固定版本 src](https://github.com/DylanZhangzzz/DevMap/tree/db696768baf366b442adf6097e0c1f4cec3f7e08/src)。本次本地读取了上述文件及 Cargo.toml；这里没有以远端页面代替本地版本检查。

## 3. 方案比较与推荐

| 方案 | 优点 | 代价 | 结论 |
|---|---|---|---|
| A. 保持多文件，封装用户入口 | 改动少，容易保留现状 | 事务、索引、并发和恢复仍散落在不同文件协议中 | 可短期改善使用，无法充分解决内部复杂性 |
| B. Rust 核心 + 仓库 SQLite + 按需共享服务 | 复用现有逻辑，集中一致性，多入口共享 | 需要可靠迁移、SQLite 依赖及进程生命周期验证 | **推荐，分阶段交付** |
| C. 全局后台 + 全局数据库 + 重写技术栈 | 跨项目查询集中 | 身份、权限、资源和故障范围都扩大，迁移成本最高 | 当前不采用 |

采用 B 不要求一开始完成 daemon：P1–P2 先用同一存储接口替换落盘，再在 P3 集中后台写入所有权。这样可以分别定位存储错误与进程协调错误。

## 4. 总体架构

![目标架构](02-devmap.svg)

逻辑依赖为：Git/宿主/用户输入 → 采集与校验 → 应用服务和领域规则 → SQLite → 查询投影 → UI/MCP/CLI。所有写命令都经过应用服务；所有读入口使用统一查询契约。UI 不读取数据库文件，也不自行推导完成状态。

| 模块 | 负责什么 | 接口/输出 | 不承担什么 |
|---|---|---|---|
| Repository resolver | 解析真实工作根、git dir、common dir，验证本机路径身份 | RepositoryContext / WorktreeDescriptor | 根据标题或相似路径猜关联 |
| Git observer | 读取 refs、HEAD、worktree 和必要拓扑；记录观察区间 | GitObservationBatch | 修改源 Git、保证观察期间 Git 不变 |
| Host adapters | 转换宿主任务、会话、活动，声明覆盖范围和来源 | NormalizedEvent / TaskObservationBatch | 把宿主报告当成经过认证的实际执行证据 |
| Application service | 命令校验、幂等、版本冲突、领域状态变化 | CommandResult / domain error | 任意 SQL 透传 |
| Repository store | 一个 SQLite 文件，事务、迁移、备份、受限查询 | 小范围读写方法 | Git 语义推断、UI 展示逻辑 |
| Projector/query service | 合并持久记录与最新观察，生成地图/任务摘要 | SnapshotEnvelope | 写入推测为事实、自动恢复撤回工作 |
| Runtime coordinator | 共享核心发现、启动、连接、退出和写入串行化 | 本地 IPC；健康状态 | 跨机器数据库共享 |
| Browser/MCP/CLI | 展示、查询或提交明确的领域命令 | 同一版本查询模型 | 独立维护另一套业务状态 |

实现继续使用 Rust；SQLite binding 作为 P1 验证的新增依赖，优先评估 rusqlite 的 bundled 构建路线并固定经过构建验证的版本。当前不承诺具体版本或所有平台已支持。保留现有打包和 npx 入口，验证依赖后再更新发布构件；不引入 Node 常驻服务作为前提。

建议的代码边界是 `store/`、`application/` 和 `runtime/` 三个小模块，加上已有 observer/reducer/adapter。这里是职责建议，不要求一次性搬动全部源文件或增加通用插件框架。

## 5. 存储位置与身份

默认路径为 `<git-common-dir>/devmap/devmap.db`。必须通过 Git 与现有路径校验解析，不能拼接当前工作区的 `.git`：linked worktree 的 `.git` 通常是指针文件。它与源码文件分离，也避免每个 worktree 都创建一份状态。

运行时可以存在 devmap.db-wal、devmap.db-shm、owner lock 及 socket；Windows 使用受本机用户权限约束的 named pipe。用户不需要手动创建、复制或删除这些文件。

数据库与后台实例按规范化 common dir 定位。P1–P2 保留旧 repository_id/worktree_id，避免迁移时无故切断历史关联。现有 ID 与路径有关：仓库移动、工作区删除后同路径重建不能默认为原身份延续。为工作区登记增加 incarnation（一次注册的生命周期标识）；路径只作为定位信息。检测到移动/重建时先提示重新关联，明确确认后记录映射，保留旧 ID。自动推断跨路径同一仓库不属于首轮目标。

使用本地磁盘数据库；不把活动数据库放在共享网络盘上供多台机器直接读写。不具备本地存储条件时明确返回不支持的存储位置，后续才评估本机用户目录覆盖选项。原有 Context Repository 的备份/共享能力不能因为移入 SQLite 就视为自动保留。

## 6. 数据模型：统一文件，明确寿命

下表是概念模型，实际 DDL 由 P1 校验。可以调整表数量，但不能丢掉以下约束。

| 表/数据集 | 主要字段 | 保留性质与规则 |
|---|---|---|
| store_meta | schema_version、repository_id、common_dir、generation | 持久；未知新版本拒绝写入，不自动降级 |
| worktrees | worktree_id、incarnation、git_dir、workspace_path、retired_at | 登记历史持久；删除工作区不级联删除事件 |
| events | event_id、source、session_id、worktree/incarnation、observed_at、event_at、source_seq、payload、payload_hash | 持久追加；重复 ID 同内容返回原结果，不同内容报冲突 |
| route_revisions | route_id、revision、request_id、start_commit、target_ref、goal、milestones、abandoned | 持久追加；唯一 route/revision 与 request_id；start_commit 不变 |
| task_binding_observations | host、task_id、worktree/incarnation、from_worktree、source、observed_at | 持久观察历史；不虚构准确移动时刻 |
| source_watermarks | source_scope、cursor/sequence、observed_at、coverage、gap_state | 持久；与接受事件/关联在同一事务更新，防旧观察回滚新状态 |
| presence_current | host/session、status、status_source、confidence、last_seen、expires_at | 派生当前态，可重建/过期；来源事件保留 |
| git_snapshots | scope、refs/HEAD、topology 摘要、scan_started_at、scan_finished_at、freshness | 当前态缓存，可刷新；不可把历史分支名当作当前 ref |
| migration_imports | import_id、source_hash、source_path、record_key、outcome、tool_version | 持久迁移溯源与幂等检查；不把损坏数据标成成功 |

原 Common Ground/Approval 对象若被导入，存为明确类型的持久记录，保留原字节、内容 ID、绑定关系和来源提交。若 SQL 库尚不能验证其原信任契约，保持 legacy-only 并通过兼容读取器访问，不降格为普通缓存。

完整对话、终端输出和源码内容不默认全部复制入库；存储原有允许的事件内容及受限引用。持久内容默认不静默删除；数据量达到预算时报告容量问题并停止接受无法可靠保存的新写入。后续归档策略需要单独说明，不能通过清空决策历史来“修复”数据库。

## 7. 写入事务和领域不变量

一条写命令遵循：识别仓库和工作区 → 验证载荷/来源 → 查幂等键 → 检查 expected_revision/水位 → 写记录及投影 → 更新 generation → COMMIT → 返回已接受。

如果在 COMMIT 后响应前断开，客户端用同一 request_id 重试并得到同一结果。提交失败不得返回成功。多次接收同一事件不得多生成一个路线版本。来源序号只在该来源范围内比较；没有序号时不能用接收顺序冒充事件发生顺序，observed_at 与 event_at 分开保存。

默认持久写连接采用 WAL、foreign_keys=ON、synchronous=FULL，并设置有限 busy timeout。不使用 CodeGraph 新建可重建索引时的 OFF 优化。FULL 也不能替代真实磁盘/故障测试；文档不承诺超出存储硬件保证的零丢失。[SQLite WAL 与持久性](https://www.sqlite.org/wal.html)

必须保留的业务不变量：

- start_commit、observed_head、merge_base 分开；后两者按观察更新。
- 计划不是已发生历史；unknown 不是一条虚构关系边。
- ahead=0 或 merged 标志不能直接等同于任务交付完成。
- 手工 cherry-pick/revert 的实际结果优先；保留撤回历史，不自动重做。
- 部分宿主任务列表不能删除未返回任务；租约超时不等于任务完成。
- delivery.mode 等计划字段不是可执行授权凭据；换库不能触发自动 Git 操作。
- 哈希帮助检查内容一致性，不证明操作者身份，也不单独提供防篡改审计。

## 8. 更新与查询契约

第一版使用按需刷新和低频核对；在 P3 再合并常驻观察。watcher/hook 只提供刷新信号。重连、checkout、ref 改变、工作区增删或观察缺口时重新核对。Git 扫描期间记录开始/结束时的相关 HEAD/refs；发生变化则丢弃候选快照并有限重试，仍不稳定就返回 pending/unknown。未提交文件状态仍是观察区间内的样本，不宣称整个 Git 仓库被原子冻结。

建议所有查询带上同一外层契约：

```json
{
  "schema_version": "devmap/query/1",
  "repository_id": "...",
  "generation": 42,
  "evaluated_at": "...",
  "scope": { "worktree_id": "...", "incarnation": "..." },
  "freshness": {
    "git": { "state": "fresh", "observed_at": "..." },
    "host": { "state": "partial", "observed_at": "..." }
  },
  "warnings": [],
  "truncated": false,
  "next_cursor": null,
  "data": {}
}
```

`fresh` 仅表示在记录的观察边界上核对成功，不保证未来时刻不变。其他状态为 pending、stale、partial、unknown、error。各来源独立标记，不能用 Git 已刷新掩盖宿主信息缺失。

同一次响应在一个数据库读事务中取得一致 generation；租约计算固定使用 evaluated_at。UI、MCP 和 CLI 若要比较一致性，必须使用相同 generation、作用域和 evaluated_at。generation 相同也不代表较晚读取时租约没有过期。分页游标绑定 generation 和查询范围；失效时要求重新开始，不混入新旧数据。

建议领域接口包括 GetMap、GetTask、GetRoute、ListEvents、Refresh、RecordEvent、UpdateRoute。前三个入口默认返回摘要，历史通过受限分页展开；无任意 SQL 端点。MCP 旧工具先由兼容层映射，保持 devmap/dock/4 消费者可用，再明确升级 schema。上述接口名称为建议，不是现有可调用工具。

浏览器首轮保持地图/详情读取；路线写入走现有已授权命令接口。后续浏览器编辑必须走相同命令处理器，并保持 loopback、token、Origin/Host 校验；不为图方便增加无保护写接口。

## 9. 后台生命周期

P3 的目标是每个仓库一个共享写入核心：客户端先解析仓库，再通过本地 IPC 查找实例。没有实例则在互斥启动协议下按需启动；握手校验协议版本、仓库身份与实例存活信息。PID 仅作为线索，不能仅凭一个存活 PID 判定归属，也不能据此结束陌生进程。

所有写入通过队列串行进入短事务，Git 扫描在事务外完成；查询通过只读事务获得快照。代理可以有多个，但不能各自启动 watcher 或写入同一个仓库。冲突版本拒绝静默并行运行第二个写核心，明确要求重连/升级。

无客户端后建议等待 5 分钟再退出，这是待测默认值。短命 hook 可以按需唤醒核心，必须等到写入确认后才报告成功；失败要返回错误与可重试身份，不丢事件后假装成功。后台离线期间不会观察到全部瞬时事件，只能下次核对当前状态并保留 capture gap。

## 10. 备份、损坏与容量

备份使用 SQLite 一致性备份接口，产生已验证快照；不要在后台运行时只复制主 .db 文件。升级和迁移前保存外部备份及哈希，备份目录放在 Git 管理目录之外，由用户指定或使用用户数据目录，避免删除仓库时连备份一起消失。[SQLite Backup API](https://www.sqlite.org/backup.html)

启动时检查 schema 和上次退出状态；发现异常执行有界检查并隔离问题。损坏的派生表可以重建，持久事件或路线损坏则停止写入、保留原文件和 WAL、报告恢复路径，不自动删库重建。schema 升级先备份并在事务内完成；旧程序看到新 schema 必须拒绝写入。

WAL 大小通过监控和 checkpoint 管理，长读事务超时/分页以避免长期阻塞回收。磁盘满、锁超时和写队列满必须有可诊断错误。默认不加入复杂的自适应缓存或查询 worker pool；只有测量证明需要后才扩展。

## 11. 兼容与简化边界

可以删除的是被 SQLite 事务和索引完整替代的多文件写入协议，以及切换后不再使用的重复扫描/投影代码。不能提前删除 legacy 读取器、事件去重、版本冲突、来源标记、捕获缺口、路径校验和完整性验证。

“不再要求 Context Repository”意味着默认本地看图和记录路线可以独立工作；旧审计对象保持可访问。是否完全移除该功能，须在现有数据依赖盘点和等价验证后另作决定。

设计自检结果：图、数据寿命、写入确认点与迁移门槛采用同一规则。P0 先确认活跃运行时与持久数据实际范围；本框架不假定旧日志全部可重放，也不将未来性能目标写成现有能力。
