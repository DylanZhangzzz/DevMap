# 第一阶段证据索引

执行合同：[两阶段验收](../superpowers/plans/2026-09-13-devmap-two-stage-acceptance.md)。本索引不等于阶段通过。当前候选为 1cc1831，CLI SHA-256：4C9109976A11D1604098A7636930E781B73AA357F62D0A51E814DE2D2E3726D9。相对 4e426e8 仅有 HTML 状态恢复修复，Rust 核心源码未变；下表的历史构件范围保持明确。

| 验收项 | 当前证据 | 证据能够支持的范围 | 剩余项 |
|---|---|---|---|
| 完整 Rust 回归 | task6-4e426e8-full-rust.log / full-rust-summary.json；root 42729 exit 0 | 58 完整组，690 通过、0 失败、10 忽略；含数据身份、幂等、迁移和共享进程专项 | 忽略项分别执行；不代替真实宿主/资源/性能 |
| 当前迁移模型一致 | stage1-native-create.log、stage1-native-export.log、stage1-native-exe-sha.txt | 冻结旧程序创建的真实旧格式，经当前 native 测试导入，完整模型对照通过；1 项，3.67 s | 当前真实 CLI 的完整运维/故障恢复说明与受控试用 |
| 旧文件保持 | stage1-storage-file-inventory.json，legacy-process-cXQcFM/manifest.json | 样例旧 13 个文件哈希相同，新增 6 个数据库/安全辅助文件 | 激活后新写入增长范围；无旧数据仓库；全磁盘与备份账本 |
| 当前渲染/交互对照 | legacy-process-cXQcFM/stage1-browser/report.json；stage1-native-browser.log；root 86248 exit 0 | 1280/560/360 px，24 组含控制组；可见差异 0，排除像素 0，最大通道差 2（阈值 2），负控制通过 | 真实运行时强杀/重连及用户打开流程；不是实时宿主截图 |
| 浏览器人工复核 | 560-candidate-workspace-details.png 已实际查看 | 展开详情页面与自动对照结果一致；不能推导所有视口无既有 UI 缺陷 | 重启选中标记基线问题单独处理 |
| 真实 CLI MCP | 当前 4C91 的 codex-host-NzWaI4/report-validated.json 与 reopen-vZ65ao/report.json | 干净仓库免手动迁移的 read→set-route→read、独立会话重开和完整路线/SQL/备份保持 | 自动 hook 与 Codex in-app Browser 使用流程 |
| owner 强杀无孤儿 | owner lifetime 的确定性窗口测试；shared-browser-UWqMyP 历史实际重启 | febbb69 专用 owner Job 修复有专项和真实重启证据；当前完整回归含专项 | 当前实际构件自然生命周期/重启与资源复验；不以强制测试清理冒充自然清理 |
| 资源/并发 | 当前完整回归的共享运行时组；早期资源审计 | 当前代码回归未出现并发/恢复失败 | 当前四客户端进程数量、总内存、十分钟核心 CPU/RSS 与空闲退出 |
| 简化账本 | [已实测文件账本](2026-09-13-simplification-ledger.md) | 明确物理文件未减少、保留证据用途 | 安装/配置/日常打开/维护动作和进程实际计数 |
| 正式第一阶段性能 | 尚无 | 现有摘要 smoke 与公共端点工具预检均不算正式人口 | 旧版 A/A 校准、预注册合同 SHA、足量同边界比较 |
| 平台与宿主范围 | 当前 Windows 本机验证 | 不声称 macOS/Linux/Claude 实际端到端通过 | 按合同列出当前 Windows/Codex 用户流程；其他范围显式未验证 |

上述 target/verification 路径均相对 worktree 根。运行构件、日志与截图仍保留在隔离验收目录，不在已安装插件中。

新增[宿主首次启动、重开与项目配置对比](2026-09-13-host-startup-and-configuration.md)：实际当前 CLI 两会话通过；adapter-parity-3Zuz9L 的新旧安装计划及干净安装配置相同，候选复用旧配置时无改写。仅覆盖项目配置步骤，不代表宿主信任和自动触发通过。

同一审计补充 inapp-99FkCv：当前真实端点在 Codex in-app Browser 打开、路线详情、手动刷新及关页重开可用，截图与 AX 树保留；测试 MCP 正常 EOF 退出且 SQL/备份保持。原指南页保留。任务观测不可用被如实显示，未以此声称真实任务/自动 hook 或自然语言开图组合流程通过。

新增[四客户端整体资源观察](2026-09-13-job-resource-observation.md)：tJ8mwv 小样例旧/新实际 Job 成员采样、四代理 PID、Git/helper 和总内存已记录。六个 Windows Job 控制测试及两个聚合负控制通过。候选额外 DevMap 的短时峰值角色仍待核验，合计 RSS 未证明减少；不能替代大规模、自然生命周期或正式性能门槛。

后续 bE0tVn 通过同负载命令行采样确认额外角色为最多四个 runtime --identity 和一个 runtime --owner；见同一资源审计补充。新证据解释同名进程来源，不倒填旧样本，也不将身份子进程从整体资源中扣除。生产构件未变，规模与完整协议/宿主门槛保持开放。

## 公共端点工具预检（不用于延迟验收）

2026-09-13，冻结旧程序 A1CFBB1C 在两工作区样例的保留旧存储上通过 process-performance.cjs 的 devmap_read_map 往返：2 冷样本、1 预热、3 热样本，脚本正常退出，MCP 子进程关闭断言通过。冷样本 1805.894/1863.271 ms，热 p95 572.412 ms，完整响应最大 11490 B。之后再次核验旧 13 个文件 SHA 全部未变。

该样例已经完成 SQL 迁移且保留旧文件，因此本次仅验证旧端点、计时工具与旧 MCP 关闭流程；不是纯旧安装环境、不是规模校准、不是旧新比较，也未独立证明整个 Windows Job 无后代或 SQL 逻辑状态保持。证据：stage1-baseline-transport-preflight.json / .log。不要拿这些数字与 20 工作区摘要 smoke 比较。

## 正式性能工具的下一步要求

现有 process-performance.cjs 只关闭 MCP 子进程；候选按需 owner 可在其后存活，故其冷样本可能复用实例。正式工具必须做到：

1. 将每个冷测量样本的进程集合独立拥有，计时从相同用户入口的进程创建前开始，直到同一公共地图响应解析/验证完成。
2. 旧版与候选使用相同负载、语义断言、计时点；不能把预先启动候选 owner 的时间移到计时之外。
3. 样本结束后以保留句柄或专属 Job 进行有界清理、确认该集合为空，才允许下个冷样本。若测量工具使用计划内强制清理，明确标记其不是自然空闲/无泄漏证据；后两者仍由独立硬门槛证明。
4. 在计时外对完整输入、SQL、冻结旧数据和临时运行目录做前后校验；未知进程/路径不可清理；失败人口及清理失败必须写报告。
5. 工具先用小样例和失败负控制验证，然后执行仅旧版规模 A/A 校准；达到合同要求并冻结门槛后才开展正式候选比较。

其中独立 Job 和计划内清理已通过[小样例预检](../../tests/browser/stage1-owned-map-preflight.md)：stage1-owned-map-WOxfcn 旧/新样本均确认 Job 清空、端点消失及 SQL/旧文件/备份保持。五项真实 Windows 负控制与正常退出检查通过。它不证明自然生命周期，单次候选冷打开比旧版慢的观察也已保留，不修改预注册上限。

四客户端公共读取预检已补齐：stage1-owned-map-oeIKdP 每版本四客户端各三次测量，独立保留样本/错误，Job、端点和数据保护检查通过；不等同单写核心进程数量验收。干净匹配的规模输入、完整语义校验与基线 A/A 校准仍待实现；没有启动正式比较，也没有冻结校准后数值。

新增[纯旧存储规模预检](../../tests/browser/stage1-legacy-scale-preflight.md)：scale-legacy-jI7MaE 具有 20 worktree、100 会话、10 万条记录，尚无 DB；生成与只读登记证据保留。T5pEW1 旧版四客户端预检通过，完整业务指纹一致，独立检查 Git 时间及逐客户端观测计数；原先严格比较失败的三个报告保留。纯旧输入准备完成，但 SQL 匹配、候选缓存/观测语义和足量 A/A 校准仍未完成。

2026-09-13 [旧版 A/A 冷/热采样](../../tests/browser/stage1-baseline-calibration.md) tkRaZE 已完整结束：40 冷启动、800 并发读取，零失败、数据保持、两个严格 Job 清空。按预先提交的区块分析，冷启动噪声区间在 ±250 ms 内，四个客户端热读取区间均超出 ±100 ms。采样成功不等于校准通过；热读取噪声、浏览器及变更可见性校准仍未关闭，未冻结正式候选合同，也未放宽上限。内存可用量较低仅是后续诊断线索，未证明延迟原因。

新增[选中状态与默认空闲验收](2026-09-13-selection-and-idle-acceptance.md)：1cc1831 完成最小 UI 修复，104 渲染测试、19 Rust UI/viewer 测试以及三视口浏览器对照通过。当前 4C91 构件在 ISHwtr 实际 owner 替换后保留选中/焦点/展开/缩放/滚动，测试进程正常退出。6D26 构件在 jupjkm 十分钟观察中自然空闲退出，核心 CPU/RSS 通过且数据保持；它支持未改动的共享核心源码，不能写成 4C91 构件实测。代理总内存、实际宿主流程与其他开放项未因此关闭。
