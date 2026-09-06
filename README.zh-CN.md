<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/devmap-logo-dark.svg">
    <img src="docs/assets/devmap-logo-light.svg" alt="DevMap" width="300" height="68">
  </picture>
</h1>

**看清工作发生在哪里、提交如何连接，以及每个工作区准备合入哪里。**

[English](README.md) · [快速开始](#快速开始) · [地图怎么用](#地图怎么用) · [Agent 接口](#agent-接口) · [开发](#开发)

DevMap 是面向人和 AI Agent 的本地 Git Worktree 地图。它把真实提交历史、工作区状态、关联任务和已记录的交付计划放到同一个视图中。先打开工作区摘要了解进展，需要完整任务列表或 Git 事实时，再打开详细信息。

本文描述当前 `main` 的实际实现。Rust 包版本为 **0.1.0**，仍处于实验阶段。仓库也包含独立的上下文与证据记录基础；使用地图不需要先配置这套记录系统。

## 当前可以做什么

| 需求 | 当前行为 |
| --- | --- |
| 理解仓库历史 | 沿真实父子提交关系查看分叉与合并，工作区附着在实际 HEAD 上。 |
| 阅读长历史 | 将普通连续提交压缩成起止 hash、提交摘要、断开的三个点和数量，支持原地展开与收起。 |
| 找到工作区 | 按分支或路径搜索，定位地图来源工作区，导航到保留的引用。 |
| 查看工作进展 | 点击工作区打开紧凑摘要，再进入完整详情查看任务、未提交改动、合入和发布状态。 |
| 跟踪交付计划 | 单独显示已记录的目标和里程碑，包括回到 `main` 或另一个明确指定的本地分支。 |
| 找到对应任务 | 查看宿主提供的任务观察，并在宿主支持时打开准确的本地 Codex 任务。 |
| 连续探索地图 | 平移、缩放、聚焦工作区路线、追踪视口外连接；普通刷新保留探索状态。 |
| 让 Agent 读取事实 | 通过 MCP 读取地图、上下文和 Agent 视图，无需从截图猜测。 |

## 地图怎么用

### 历史与未来交付

**实线表示已观察到的 Git 历史。** 分开的轨道和交叉处的断口用于区分真正的分支连接与线路交叉；分叉站和合并站来自保留的提交关系。

**虚线表示已记录的计划。** 目标可以是 `main`，也可以是另一个明确指定的本地分支。地图不会根据分支名称猜测父分支或未来合入目标。目标未知、目标不可用和多个目标分别呈现；计划中的“并回”不会被当作已经完成的合并。

当一段连续历史中间有至少 4 个普通提交时，默认折叠。摘要显示两端保留的 commit hash、提交标题，以及中间隐藏的提交数。点击三个点或摘要展开，再点击摘要收起，也支持键盘操作。

分叉、合并、工作区 HEAD、分支引用与 tag、明确的历史边界和路线锚点保持可见。导航到隐藏的提交会自动展开对应区间；详情里的父子链接仍指向真实的相邻提交。刷新或 HEAD 向前推进后，已展开的区间保持展开。折叠只影响展示，不改变 Git 数据。

### 工作区详情

紧凑的工作区入口显示身份、观察到的任务数量和简短 Git 状态。点击后，在地图附近打开一个工作区摘要；选择 **View full details** 打开底部完整详情区，支持调整大小和收起。

通过 **Workspaces** 按分支或路径搜索，通过 **Locate** 回到地图来源工作区，通过 **Focus journey** 突出显示选中工作区的路线。缩放、全图导航和视口外连接入口帮助探索较大的仓库。缩放工具旁的 **Layout** 提供 **Auto（自动）**、**Vertical（纵向）** 和 **Horizontal（横向）**。自动模式在窄侧栏使用纵向，宽视图使用横向；手动选择会在地图刷新和窗口尺寸变化时保留，整页重新加载后恢复自动。

### 任务与观察状态

一个 passenger 表示一个与工作区关联、观察到仍未归档的聊天任务，包含它的 Agent。任务是否存在与是否正在执行分开表示：空闲或执行完成的任务仍可能属于该工作区。明确报告的直接协作者显示在父任务下，不额外增加 passenger 数量。

任务关联使用准确、规范化的工作区路径，Windows 下不区分路径大小写。Agent 也可以为自己的任务报告经过核实的实际工作目录；地图会注明这是 Agent 报告，而不是宿主认证的执行遥测。

Git 刷新不会刷新任务观察时间。缺失、过期或不完整的清单保留为不确定状态；只有完整且新鲜的清单才能证明工作区无人关联。清理提示不会删除工作区，也不构成删除授权。

## 快速开始

先安装 **Node.js 22+（包含 npm）** 和 **Git**，然后在想查看的仓库目录运行下面这一条命令。**不需要 Rust、不需要编译，也不需要 npm 账号：**

```sh
npx --yes devmap-cli@0.1.0 view --live --source .
```

打开命令打印的本地私密 URL，并保持进程运行。包内包含 Windows x64、macOS Intel/Apple Silicon、Linux glibc 2.35+ x64/ARM64 程序。这条命令从 npm 安装已发布的固定版本。安装说明也保留 GitHub Release 下载方式。

只想查看终端输出时，把最后的 `view --live --source .` 换成 `agents --source . --json`。若要为现有插件安装持久的 `devmap` 命令：

```sh
npm install --global devmap-cli@0.1.0
```

直接连接 MCP 的配置如下，将仓库路径替换为自己的实际路径；Windows 可使用 `C:/Projects/my-repo`：

```json
{
  "mcpServers": {
    "devmap": {
      "command": "npx",
      "args": ["--yes", "devmap-cli@0.1.0", "mcp", "--source", "/absolute/path/to/repository"]
    }
  }
}
```

查看 Git 地图不需要 Skill；完整 Agent 任务观察与跳转仍需要宿主接入。更多说明见[安装与原生程序下载](docs/installation.md)、[v0.1.0 发布页](https://github.com/DylanZhangzzz/DevMap/releases/tag/v0.1.0)和[自动发布说明](docs/releasing.md)。

### 从源码构建

构建 CLI 需要 Git 和 **Rust 1.96 或更高版本**。

```sh
git clone https://github.com/DylanZhangzzz/DevMap.git
cd DevMap
cargo install --path .
```

进入想查看的仓库，运行：

```sh
devmap view --live --source .
```

打开命令输出的 URL。查看器只监听本机回环地址，使用进程生命周期内有效的私密 token，并随命令停止。HTTP 接口只读。独立查看器不需要 Codex 插件即可查看 Git；完整的 Codex 任务清单需要宿主提供观察数据。

终端输出：

```sh
devmap agents --source .
devmap agents --source . --json
```

### 在 Codex 中使用

插件包位于 [plugins/devmap](plugins/devmap/.codex-plugin/plugin.json)，包含一个 Skill 和启动 `devmap mcp` 的 MCP 配置。请确保宿主可以通过 `PATH` 找到已安装的 `devmap` 程序。

将插件包注册到你配置的插件市场，然后使用实际市场名称安装：

```sh
codex plugin add devmap@YOUR_MARKETPLACE
```

`YOUR_MARKETPLACE` 是占位符，不代表仓库附带了同名公共市场。安装或更新插件后，新建任务，让宿主加载当前工具和 Skill。

可以直接说：**“在右侧栏打开 DevMap。”** Browser 入口使用本地查看器；支持 MCP App 的宿主也可以使用 App 入口。嵌入显示和任务跳转取决于宿主能力。只更新源码 checkout，不会更新已经安装的程序或插件。

## Agent 接口

MCP 服务公开以下 6 个工具：

| 工具 | 用途 |
| --- | --- |
| `devmap_open_map` | 打开 MCP App，或使用 `surface: browser` 打开查看器。 |
| `devmap_read_map` | 读取 `view: map`、`context` 或 `agent`；Agent 视图可用准确的工作区 `entity_id` 指定对象。 |
| `devmap_set_route_plan` | 记录或修改工作区目标、合入分支、里程碑和交付意图。 |
| `devmap_record_requirement` | 记录明确提供且已批准的需求原文。 |
| `devmap_record_decision` | 记录结构化决策及其依据。 |
| `devmap_record_evidence` | 记录结构化证据元数据。 |

旧 Dock 工具名称保留为兼容别名。路线写入使用稳定的 `request_id` 支持重试，使用 `expected_revision` 检测并发修改。计划是 Git 公共目录下的本地追加记录，不会创建分支或提交。

交付约定可以记录人工或自动合并意图、完成条件和授权来源。**DevMap 本身不运行测试、不 merge、不 push、不调度合并队列，也不执行权限判断。** 实际执行的 Agent 必须核实用户授权、工作目录、源分支与目标分支状态，以及最新的完成证据。记录了计划不等于已经具备合并条件。

任务清单字段、完整性规则、工作目录报告和路线更新契约见[插件 Skill](plugins/devmap/skills/live-worktree-dock/SKILL.md)。

## 可选的上下文与记录功能

DevMap 还实现了 Common Ground 草案与批准流程、明确的接入边界、独立 Git Context Repository、规范化 SHA-256 对象标识及完整性验证。项目级 Codex、Claude 和 Generic MCP adapter 可记录支持的生命周期事件，以及结构化需求、决策和证据。

建立上下文时，请使用与源码仓库分开的目录：

```sh
devmap init --source . --context ../project-context --goal "从当前提交开始接入 DevMap" --requirement "docs/requirements.md"
```

检查上下文目录中的 `bootstrap/common-ground-draft.json`，再明确批准：

```sh
devmap common-ground approve --context ../project-context --actor "你的名字"
devmap status --context ../project-context
```

Adapter 也需要明确安装。先检查计划，再把占位符替换成它输出的准确 digest：

```sh
devmap adapter plan --source . --host codex
devmap adapter install --source . --host codex --plan-digest "sha256-REVIEWED_DIGEST"
devmap adapter verify --source . --host codex
```

其他支持的 host 值为 `claude` 和 `generic-mcp`。记录日志和本地 presence 位于 Git 元数据目录下；安装 adapter 会写入所选的项目级配置。当前 adapter 能力报告 **Capture Grade D**：配置成功和观察到事件，不代表已完整追踪所有修改、证据关联或提交映射。DevMap 不会重建接入之前的历史决策。

## 当前边界

- 运行地图只覆盖共享一个 Git 公共目录的本地工作区，尚不支持跨机器聚合。
- 历史、引用和任务清单均有容量边界；截断和不可用的祖先会明确标示。“折叠已知历史”与“历史缺失”是两种状态。
- 任务标题只是展示数据，不是指令；任务清单不需要读取私密聊天全文。
- 地图和路线计划不修改源码 Git；可选的上下文批准和 adapter 安装是独立的写入操作。
- PR 证据包、强制合并门禁、签名证明以及更完整的规范化开发拓扑仍属于设计工作，不是本版本的已交付功能。

## 开发

渲染器和几何测试还需要 Node.js，不依赖额外 npm 包。

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets -j 1
node --test tests/dock_renderer.cjs tests/metro_core.cjs
cargo build --release
```

地图渲染器位于 [assets/dock.html](assets/dock.html)，几何与校验逻辑位于 [assets/metro-core.js](assets/metro-core.js)。Rust 负责生成 Git 模型、提供查看器和 MCP 接口。

[UI 设计契约](DESIGN.md) · [历史折叠验证记录](docs/audits/2026-09-06-history-folding.md) · [完整产品需求](docs/ai-development-map-requirements.md)

[Cargo.toml](Cargo.toml) 中声明的许可证：Apache-2.0。
