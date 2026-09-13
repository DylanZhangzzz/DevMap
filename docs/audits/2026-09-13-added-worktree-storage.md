# 激活后新增 worktree 的实际存储检查

当前 25BF 构件（生产源 5dd8288）在 linked-storage-dfQYX8 中通过实际 CLI 完成以下操作：冻结旧程序写入一个主工作区会话，候选迁移至仓库级 SQL，然后通过 Git 新增一个 linked worktree，候选在新 worktree 写入开始和结束事件。

主工作区和新 worktree 的 storage inspect 指向同一物理数据库，即主仓库 .git/devmap/devmap.db。新 worktree 没有执行 storage migrate、adapter install 或手动启动服务；两个 native hook 命令自动找到共享存储。新 worktree 的 Git 管理目录下没有 DevMap 文件，未产生独立数据库、events.ndjson、presence 或路线 JSONL。

最终 SQL 包含两个 journal session、三条 journal record、两个 worktree registry 条目，generation=2。新增会话的开始/结束事件均归属新增 worktree，顺序正确且哈希相连；从 linked worktree 执行 storage verify 返回 verified=true。主工作区全部原 legacy 文件 SHA 及冻结备份保持不变，说明本例新写入进入 SQL，而不是继续追加旧日志。

证据：target/verification/linked-storage-dfQYX8/report.json、main-inspect.json、linked-inspect.json、new-start.json、new-end.json、verify.json。测试脚本为 tests/browser/linked-worktree-storage.cjs，固定新旧可执行 SHA，并只创建新的隔离样例。先前 hMgu0K 在只读查询阶段遇到 Python SQLite 不接受 Windows 扩展路径 URI；保留失败报告，改用已有受检规范路径后在新样例通过。该失败不证明生产库损坏。

这是存储接入证据，不能把显式 native hook 调用包装成宿主自动触发。测试没有在新 worktree 创建 .codex 配置，也没有证明真实 Codex 会发现父工作区的项目 hook。全局/仓库/worktree 的宿主配置继承和信任仍须另验；此处不关闭完整“日常操作不增加”门槛。也不宣称运行时所有文件只剩一个 DB，迁移备份、主库 sidecar 和运行时文件继续保留。
