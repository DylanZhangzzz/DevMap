# 当前候选宿主启动与配置证据

候选为 1cc1831 / SHA-256 `4C9109976A11D1604098A7636930E781B73AA357F62D0A51E814DE2D2E3726D9`，实际 Codex CLI 为 0.154.0-alpha.6.2。所有样例在本 worktree 的 target/verification 内；临时状态和自动备份位于专用测试根。未修改用户项目、全局配置、信任记录或已安装插件。

## 干净仓库首次使用

codex-host-NzWaI4 使用新增的 DEVMAP_HOST_EMPTY_START=1 模式，启动前确认没有 .git/devmap，未运行 storage migrate。实际 CLI 仅调用 read_map → set_route_plan → read_map，返回同一仓库和工作区的 revision 1 路线；退出码 0，验证器与错误路线负控制通过。首次写入自动初始化 SQLite，独立 startup-sql.json 确认 schema 2、active 状态以及一条路线和来源关联。

证据：stage1-4c91-host-empty.log；codex-host-NzWaI4/{manifest.json,events.jsonl,report-validated.json,startup-sql.json}。这证明干净仓库首次路线写入无需手动数据库初始化，不代表有旧数据的升级或宿主注册/信任没有步骤。

## 关闭后在另一实际 CLI 会话重开

codex-host-reopen.cjs 在确认仓库命名管道不存在后，启动独立只读 CLI 会话，仅允许一次 read_map。新 thread ID 与首次不同；完整路线对象（含版本、时间、来源和起点）、仓库及工作区身份完全一致。全部 14 张表的逻辑内容/结构和自动备份清单前后相同。退出码 0，超时为 false。

证据：stage1-4c91-host-reopen.log；codex-host-NzWaI4/reopen-vZ65ao/{events.jsonl,report.json}。验证器两项控制测试通过，错误仓库、工作区、目标、版本、时间、缺失和额外路线均不能通过。CLI MCP 重开已验证；不据此声称桌面 Browser 导航或自动 hook 已通过。

## 新旧项目 hook 安装对比

adapter-parity-3Zuz9L 中，冻结旧程序 A1CF 与候选在同一干净仓库生成字节一致的安装计划。旧程序安装后，候选再次安装返回 changed=false，配置字节不变；另一个干净仓库的候选新安装也生成相同配置。

两版均为一次 adapter plan 加一次携带已审阅 digest 的 adapter install；产物均为一个项目 .codex/hooks.json，10 个事件、3067 字节，SHA-256 `441b9a5e9b38d4a7ea1b151811dc80895119d48e5c0978647f84806265cf64cc`。这些是 DevMap CLI 配置步骤，未测量下载、宿主注册和信任审核步骤。

证据：adapter-parity-3Zuz9L/{plan-report.json,install-report.json,fresh-install-report.json} 及同目录原始计划/安装/verify 输出。候选 verify 为 configured=true、activation_verified=false；实际自动触发、会话正确归属和事件落库仍是开放硬门槛。
