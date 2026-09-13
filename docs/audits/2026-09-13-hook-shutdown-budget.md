# Codex 结束事件超时缺口与修复

官方 [Hooks 文档](https://learn.chatgpt.com/docs/hooks)规定 SessionEnd 默认 1 秒、最高 3 秒；非托管 hook 的定义变化需要重新审核信任。此前候选和冻结旧版生成的 SessionEnd 都没有显式 timeout，配置相同不代表真实宿主能够完成结束记录。

4C91 在 hook-budget-u3oavT 新建仓库中直接运行三组 SessionStart/SessionEnd 原生命令。结束命令分别耗时 1866.9097、1532.6188、1642.0067 ms，全部退出 0，但全部超过默认 1000 ms。这是实际命令耗时，未启用宿主 hook，也未模拟宿主 1 秒强杀；不能声称观察到了真实宿主丢记录。首个开始命令为 3710.8652 ms，说明冷路径也不能直接假定小于 3 秒。

源代码提交 5dd8288 为 Codex SessionEnd 显式生成 timeout=3；其他事件和 Claude 配置不变。迁移识别此前 DevMap 精确生成的无 timeout 结束组，升级时不留空组或重复 handler；用户自定义组元数据仍保留。21 个 adapter_install 检查通过，含新配置、旧配置升级、幂等、安全覆盖和自定义配置保留。初次回归有一个检查仍限定 handler 恰好三个字段，已更新为仅允许 Codex SessionEnd 多出经过断言的 timeout=3 字段；失败日志不能当作生产运行失败。

这是一项实际配置变化，因此早前“新旧安装文件字节一致”和“再次安装无改写”的结论只适用于 4C91。新版安装计划 digest 会变化，用户应按正常宿主流程审核新定义；未替用户修改现用项目或全局信任。新增一次审核是否符合第一阶段“不增加日常操作”的完整接入合同仍须在阶段报告解释，不能隐藏这项升级成本。

尚未关闭：当前新构件实际宿主自动触发、SessionEnd 的冷启动/高负载/宿主终止边界和落库确认。3 秒是宿主支持的最大窗口，不是当前代码所有情形都满足的保证。不使用绕过信任参数，也不以直接运行命令冒充宿主触发。核心身份校验和持久化确认没有因本修复被跳过。

原始命令和时间在 target/verification/hook-budget-u3oavT/report.json，session 61154 退出 0。测试属于原生命令路径检查；SQLite 完整记录和实际宿主身份须另外核对。

新 release 构件：target/verification/task6-candidate-5dd8288/devmap.exe，SHA-256 25BF6631EF7387E524744A1B2B8CCE01B76D54C58CDB897353ACC5D042DFADEC；构建退出 0，36.59 秒。2 个 adapter_conformance 检查亦通过（43.64 秒），与 21 个安装检查共 23 项。实际新 CLI 在 u3oavT 仓库完成审阅计划对应的安装，生成 timeout=3；verify 返回 configured=true、activation_verified=false，原始输出 new-adapter-install.txt / new-adapter-verify.txt 保留。未触发自动 hook、未修改全局配置/信任或已安装插件。旧版 4C91 命令耗时不能写成新构件的宿主性能结果。

## 新构件空闲退出后的结束事件

hook-cold-end-OHStk1 使用固定 25BF 构件，在新仓库接受 SessionStart 后等待 75 秒，期间不探测运行时。随后单次连接确认命名管道不存在，再发送 SessionEnd 原生命令。开始耗时 3725.7815 ms，结束耗时 1690.0143 ms；两次退出均为 0，session 91953 正常结束。此次结束命令落在 3 秒内，不是百分位或高负载保证，也没有模拟宿主截止时强杀。

只读 SQL 核对恰好一条 session_started 和一条 session_stopped：会话、宿主、仓库/工作区上下文及 actor 一致，sequence 为 1/2，第二条 previous_sha256 指向第一条。随后实际 storage verify 返回 active、generation=2、verified=true。结果在 report.json 和 storage-verify.json；两项验证器控制测试通过，错误会话/仓库/actor、缺失结束、错误顺序和哈希关联不能通过。增强验证器也已重放实际保存记录，无需重复启动场景。

实际自动 hook 仍需正常宿主信任审核。已向用户请求仅针对 u3oavT 隔离仓库、固定 25BF 候选的审核例外，因为两阶段合同原先禁止修改全局信任。未获得回复前不写信任记录、不启用绕过参数；这项请求不等同已获授权，其他独立验收可继续。
