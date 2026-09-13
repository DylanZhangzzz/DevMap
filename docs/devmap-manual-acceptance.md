# DevMap 简化版：代码交付，待人工验收

按 2026-09-13 用户修订，本次交付可运行代码，最终验收由用户完成。自动性能校准不再阻塞交付，已有未通过结果保留。

## 构件与代码

- 分支：`codex/devmap-sqlite-compatible`。
- 后端基线：`5dd8288`；当前候选叠加本次工作区详情布局、复制按钮和分支遗漏 UI 修复。详见[分支核对记录](audits/2026-09-13-visible-workspace-branch-review.md)。
- Windows 程序：`target/verification/commit-steady/devmap.exe`，大小 9907712 字节。
- SHA-256：`685B74848617E1E8D85A2B0FFA0E51ACFB095ED273CD7C5484FDFEE5066BECD8`。
- `--version` 为 `devmap 0.1.1`，与旧版相同；区分候选请用路径和哈希。

已实现仓库共用 SQLite、按需共享核心、旧存储受控迁移/验证/备份、幂等与恢复保护、兼容全量地图和分页摘要。网页保持原有界面，另修复刷新/重连时选中状态保持。Codex SessionEnd 安装定义显式使用 3 秒超时。

## 启动查看

在 PowerShell 中执行，最后的仓库路径换成希望查看的本地 Git 仓库：

```powershell
& 'C:\Users\user\Documents\ChatGPT\AI auto-git context\.worktrees\devmap-sqlite-compatible\target\verification\commit-steady\devmap.exe' view --live --source 'C:\path\to\repository'
```

保持该终端运行，将输出的完整本地 URL 打开到 Codex 内嵌浏览器。结束本次查看时在终端按 Ctrl+C。此命令不替换已安装插件。原有指南页可继续保留。

仅查看不会自动迁移已有旧存储，也不保证创建 DB；因此“网页已打开”不等于“该仓库已切换 SQLite”。先用同一个程序执行 `storage inspect --source <仓库路径>` 查看所用后端。干净仓库在首个有效共享写入时自动初始化，已有旧数据按[存储操作说明](sqlite-storage.md)受控迁移。迁移会改变后端选择，试用时先选可丢弃的测试仓库，不应在旧写入器仍运行时切换真实仓库。

## 建议人工检查

| 检查 | 关注点 |
|---|---|
| 页面与操作 | 对比原网页布局、工作区与任务详情；缩放、滚动、展开、选中、焦点和刷新是否符合原有习惯 |
| 数据刷新 | 修改测试仓库文件或提交后，地图状态是否及时更新，旧内容是否明确显示为过期或未知 |
| 重开与多窗口 | 关页重开、多个客户端同时读取，是否仍显示正确仓库、任务与已保存路线 |
| SQLite 与迁移 | 测试仓库的 inspect/verify、迁移备份、重开后的记录与路线是否正确；运行期允许 WAL/SHM 和锁文件 |
| 实际宿主 | 通过宿主正常配置与信任审核后，验证 hooks 是否自动触发、结束记录是否落库；目前没有冒充这一步已经通过 |
| 体验与性能 | 由你判断打开、刷新、变更可见时间及机器资源占用是否可接受 |

无需一次做完上述所有项。反馈时附仓库场景、操作步骤和实际现象即可；收到具体问题后继续修复。

## 已有检查及已知限制

已有完整 Rust 回归在 `4e426e8` 上为 690 通过、0 失败、10 忽略；之后 UI 变更有 104 项渲染和 19 项 Rust UI/viewer 检查，SessionEnd 配置变更有 23 项相关检查。共享核心源码未因后续诊断改动。当前构件已核验哈希、版本，当前工作树 `cargo fmt --all -- --check` 通过。不把历史测试说成当前构件又做了一遍。

真实浏览器对照、owner 重启、迁移、幂等、数据保护和 CLI MCP 已有专项记录，详见[证据索引](audits/2026-09-13-stage1-evidence-index.md)。这些提供交付依据，不能替代你的人工结论。

性能尚未证明达标：旧版 A/A 热读取及两次变更校准噪声超出原预算，没有正式旧新性能比较，原绝对性能目标也未全部关闭。Git 启动链诊断未进入产品代码，不要求用户修改 PATH。

真实自动 hooks、完整宿主接入及 macOS/Linux/其他宿主端到端仍未完整验证。新增旁侧 worktree 的项目 hooks 配置不会因主工作区已配置就自动成立；新旧版本均存在该边界。SessionEnd 的 3 秒配置不保证所有高负载情况均可完成。

SQLite 不是运行时只有一个文件；旧原件、备份、WAL/SHM、安全元数据保留。已接受 SQL 新写入后，没有承诺通用无损降级到旧版。分歧历史须按存储说明诊断和前向恢复。

当前状态：代码可供人工试用；用户尚未给出验收结论。未自动合并、推送、安装、发布或切换真实用户库。

原 25BF 构件的交付核验记录：`target/verification/manual-handoff-TqU0GL/report.json`。当前界面构件的浏览器检查：`target/verification/manual-details-final/browser-check.json`。该记录确认生产源码与构件来源对应、程序哈希/版本、四个 CLI 入口帮助和本说明的本地链接；没有将其当作功能或性能验收。

滚动修复：聊天卡和展开详情按内容自然增高，由地图统一承接滚轮；显示更多聊天仍为显式操作。105 项渲染检查、97 项几何检查通过。420/900 px 真实浏览器中，在聊天与详情上分别滚动 180 px 均移动外层地图 180 px，内部没有溢出或滚动截留。证据：target/verification/scroll-preview/browser-check.json。

控件优化：106 项渲染/交互检查通过；当前构件在 420/900 px 浏览器中验证三种字段复制、刷新中的反馈保留、键盘展开、缩放/方向/定位/平移、分支 HEAD 详情、详情尺寸与折叠、搜索关闭、44 px commit 点击区域与焦点返回。无页面脚本错误；HTML 200662 字节，保留 196 KiB 上限。详见 [控件检查](audits/2026-09-13-controls-review.md) 和 target/verification/button-final/browser-check.json。

Commit 详情优化：选中标记缩小到节点本身，44 px 点击区域保留；详情按节点位置与地图边界定位并显示连接线，窄屏限制高度以保留节点，标题/哈希/操作支持换行。106 项渲染检查通过；973/420 px 实际浏览器没有脚本错误或横向溢出，详情位于地图内。证据：target/verification/commit-final/check.json 与两张 commit 截图。当前 HTML 203337 字节；新增定位/连接逻辑后资源预算从 196 KiB 调整为 200 KiB，没有删除源码注释来规避预算。该构件待用户人工验收。

点击 commit 稳定性修复：移除对已点击节点再次 locate 的滚动，同步计算面板位置后转移焦点；窄屏根据节点上下空余空间限制面板高度。106 项渲染检查通过；973/420 px 分别记录 199/205 帧，点击及后续刷新期间页面和地图滚动坐标不变，连接线可见、无横向溢出或页面脚本错误。证据：target/verification/commit-steady/check.json。当前预览待人工确认实际侧栏手感。
