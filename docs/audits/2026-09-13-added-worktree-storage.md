# 激活后新增 worktree 的实际存储检查

当前 25BF 构件（生产源 5dd8288）在 linked-storage-dfQYX8 中通过实际 CLI 完成以下操作：冻结旧程序写入一个主工作区会话，候选迁移至仓库级 SQL，然后通过 Git 新增一个 linked worktree，候选在新 worktree 写入开始和结束事件。

主工作区和新 worktree 的 storage inspect 指向同一物理数据库，即主仓库 .git/devmap/devmap.db。新 worktree 没有执行 storage migrate、adapter install 或手动启动服务；两个 native hook 命令自动找到共享存储。新 worktree 的 Git 管理目录下没有 DevMap 文件，未产生独立数据库、events.ndjson、presence 或路线 JSONL。

最终 SQL 包含两个 journal session、三条 journal record、两个 worktree registry 条目，generation=2。新增会话的开始/结束事件均归属新增 worktree，顺序正确且哈希相连；从 linked worktree 执行 storage verify 返回 verified=true。主工作区全部原 legacy 文件 SHA 及冻结备份保持不变，说明本例新写入进入 SQL，而不是继续追加旧日志。

证据：target/verification/linked-storage-dfQYX8/report.json、main-inspect.json、linked-inspect.json、new-start.json、new-end.json、verify.json。测试脚本为 tests/browser/linked-worktree-storage.cjs，固定新旧可执行 SHA，并只创建新的隔离样例。先前 hMgu0K 在只读查询阶段遇到 Python SQLite 不接受 Windows 扩展路径 URI；保留失败报告，改用已有受检规范路径后在新样例通过。该失败不证明生产库损坏。

这是存储接入证据，不能把显式 native hook 调用包装成宿主自动触发。测试没有在新 worktree 创建 .codex 配置，也没有证明真实 Codex 会发现父工作区的项目 hook。全局/仓库/worktree 的宿主配置继承和信任仍须另验；此处不关闭完整“日常操作不增加”门槛。也不宣称运行时所有文件只剩一个 DB，迁移备份、主库 sidecar 和运行时文件继续保留。

## 项目 adapter 配置边界

linked-adapter-Pvxl6B 使用冻结旧版 A1CF 与当前候选 25BF，各自在新的隔离仓库执行主工作区 adapter plan/install，然后创建旁侧 linked worktree。两者主工作区 verify 均为 configured=true；新增 worktree 均返回退出码 1、configured=false、activation_verified=false，缺少相同的 10 个绑定。新 worktree 的 plan 指向其自身 .codex/hooks.json，不是主工作区配置。检查后新增工作区没有 .codex 目录，主配置字节保持不变。

这是现有原生 adapter 的工作区配置边界，不是新增候选回归，也不是实际 Codex 宿主发现规则的完整验证。该接入模式不能仅靠“主工作区已安装”保证新增 worktree 自动采集；安装插件或其他宿主配置层是否提供覆盖仍待实际验收，不能用手动调用 native hook 替代。没有在新 worktree 安装配置，没有更改宿主信任或启用自动 hook。

两版本安装后主 Git 数据目录均只有 adapter-install.lock；这个配置锁应计入文件账本，本测试未初始化业务 DB。脚本 tests/browser/linked-worktree-adapter.cjs 固定新旧 SHA，各命令保留退出码、stdout/stderr，最终 report.json 为 passed=true。先前 aulcXq 将预期的 verify 退出码 1 当执行异常；UXyHXd 错误假设 adapter install 不创建数据目录，实际仅为配置锁。两个失败报告保留，修正测试假设后在全新 Pvxl6B 样例通过；没有因此修改生产代码。
