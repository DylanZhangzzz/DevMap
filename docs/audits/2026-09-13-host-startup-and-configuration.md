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

## Codex 内置 Browser 实际打开、刷新和重开

同一 codex-host-NzWaI4 数据通过当前 native MCP 的 devmap_open_map 返回真实本机 HTTP 地址，再由 computer-use 在 Codex in-app Browser 新标签打开。实际展开 Route plans 并进入详情，看到原路线目标、revision 1、完整起点和来源。手动 Refresh map 后详情和路线版本仍在，页面明确显示只刷新 Git、未同步任务。关闭测试标签再打开同一地址，路线详情仍可读取。

证据位于 codex-host-NzWaI4/inapp-99FkCv：opened.json、closed.json、before-refresh.ax.txt、after-refresh.ax.txt、reopened.ax.txt，以及 route-details.png、after-refresh.png、reopened.png。路线详情截图已人工查看。关闭两张测试标签后，Browser 仅保留原来的 guide.html；测试 MCP 以 stdin EOF 正常退出（code 0、signal null），全部 SQL 逻辑状态和自动备份清单保持。

本轮是实际 in-app Browser 对候选端点的可用性观察，没有注入测试 HTML、截图遮罩或模拟后端。它不是新旧同视口像素比较，也没有通过 Codex 的自然语言工具选择完成开图；该组合流程仍待验证。任务列表未注入，页面如实显示 Task observation unavailable，因此不能据此关闭真实任务同步、自动 hook、Agent 归属或全流程宿主验收。浏览器重开时 MCP 仍运行，也不等同 owner 重启证据。
