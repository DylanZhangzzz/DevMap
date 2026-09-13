# 当前候选 CLI 迁移与故障操作演练

候选仍为 1cc1831 / SHA-256 4C9109976A11D1604098A7636930E781B73AA357F62D0A51E814DE2D2E3726D9。本轮只修改测试与文档。冻结旧程序 A1CF 在新建 legacy-process-4SjQ71 中生成两个 worktree、两个会话、路线和证据；未使用真实用户库，也未污染此前性能样例。

storage-operator-rehearsal.cjs 要求未迁移、属于本 worktree 验收根的样例，核对两版可执行 SHA，使用独占 operator-rehearsal 目录。其完整真实 CLI/MCP 演练退出 0（session 80823），report.json passed=true。

| 实际动作 | 已核实结果 |
|---|---|
| inspect / 无库 verify | inspect 返回 legacy，verify 拒绝无库；两者均未激活 DB |
| migrate 到新的外部冻结目录 / verify | 返回 active、verified=true，原旧文件 SHA 不变 |
| 相同冻结路径重复 migrate | SQL 所有 14 张表逻辑摘要及 schema 保持，没有重置数据 |
| MCP 新路线写入及同请求重试 | 原路线 revision 2 升为 3，目标为 Accepted after controlled SQL migration；重试返回完全相同记录 |
| 新路径一致性 backup | 备份全部表/结构与已接受新记录的 SQL 状态相同；原旧文件仍未增长 |
| 重复目标 backup | 被拒绝，已存在备份 SHA 未变；本次命中已有 sidecar 防覆盖检查 |
| 冻结旧程序再写一条事件 | 新版 verify 和 migrate 均拒绝 legacy source drift；不是悄悄改回旧后端 |
| 拒绝之后检查两份历史 | SQL revision 3 等完整逻辑状态保持，旧 journal 中迟到事件仍在，冻结目录及一致性备份不变 |

所有原始命令 stdout/stderr、MCP JSONL、SQL 前后摘要和备份 SHA 均在 target/verification/legacy-process-4SjQ71/operator-rehearsal，路径相对当前 worktree。该样例现已故意产生旧写入分歧，必须保留，不能用于干净迁移或性能输入。不要重新运行此脚本指向同一样例。

原始错误明确为：legacy source changed after activation; SQL and legacy retained, forward recovery required。这里证明的是检测与保留，**不是完成两份分歧历史的自动合并**。现有 CLI 没有通用恢复/逆向导出命令；操作指南应让用户保留状态进入诊断，而非删除 DB、锁、sidecar 或激活标记后重试。

backup 成功输出中的 verified=false 来自随后执行的 inspect；源码 backup_to 已执行 SQLite 一致性复制、integrity_check 和 foreign_key_check。本轮另用只读 SQL 全表摘要确认副本与源一致。已在 [SQLite 操作指南](../sqlite-storage.md) 明确区分退出状态、备份校验和 storage verify 的业务/来源校验。

未覆盖：激活中间时点中断的当前 CLI 注入、通用前向合并、真实用户受控试用、宿主自动 hook 升级与关闭、采样外全部进程生命周期。相应专项/宿主门槛仍独立验收，不能把这次操作演练写成全部恢复能力已达标。
