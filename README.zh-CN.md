<p align="center">
  <img src="docs/assets/devmap-topology-vision.svg" alt="DevMap 产品愿景：连接需求、决策、代码与证据的共享拓扑图" width="100%">
</p>

<h1 align="center">DevMap</h1>

<p align="center"><strong>供人类与 AI Agent 共同使用的、可验证的开发地图。</strong></p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="docs/ai-development-map-requirements.md">产品需求</a> ·
  <a href="#快速开始">快速开始</a> ·
  <a href="#路线图">路线图</a>
</p>

<p align="center">
  <img alt="项目状态：实验阶段" src="https://img.shields.io/badge/status-experimental-E9A23B">
  <img alt="当前里程碑：Phase 1B" src="https://img.shields.io/badge/milestone-Phase%201B-6C63FF">
  <img alt="核心语言：Rust" src="https://img.shields.io/badge/core-Rust-CE422B?logo=rust">
  <img alt="捕获等级：取决于宿主" src="https://img.shields.io/badge/capture%20grade-host--dependent-2F81F7">
</p>

> [!IMPORTANT]
> DevMap 正在积极开发中。Phase 1A 提供 Common Ground 与完整性基础；Phase 1B 新增本地 Codex、Claude Hooks 和 Generic MCP 捕获端点。Live Worktree Dock MVP 新增本地 worktree 与已接入 Agent 的只读视图。源码 Git 工作流自动化、PR 证据链和上图中的完整交互式拓扑 Viewer 尚未实现。

## 问题背景

大型长期功能不再只有一个作者，也不再只存在于一次对话中。开发者和多个 AI Agent 会跨分支、Session、Pull Request 与持续变化的需求并行工作。Git 保存了代码，却通常没有保存代码形成的路线。

DevMap 的目标，是为 PM、研发负责人、开发者和 Agent 提供一张统一、可查询的开发地图。这张地图必须回答六个问题：

| 问题 | 它揭示什么 |
| --- | --- |
| 为什么走这条路线？ | 背后的需求、约束或推理 |
| 这是人类指令还是 Agent 自主选择？ | 路线是被要求的，还是自主选择的 |
| Agent 是否有权做这个决定？ | 该选择对应的权限策略与批准边界 |
| 哪些方案被放弃？ | 曾被认真考虑但没有采用的岔路 |
| 哪项证据证明它有效？ | 与代码绑定的测试、构建、评审和其他证据 |
| 这个决定是否已经被替代？ | 该决定当前是否仍然有效 |

目标不是保存每一句聊天，而是在每个有意义的开发岔路口，保存最小但完整的证据链。

## DevMap 如何工作

Phase 1A 为已有项目建立明确的共同起点：

```mermaid
flowchart LR
    S[Source Repository] -->|只读检查| D[Common Ground 草案]
    D -->|人类评审| A[明确批准]
    A --> C[Context main 中的 Canonical Objects]
    C --> V[独立完整性验证]

    subgraph Context Repository
        D
        A
        C
        V
    end
```

DevMap 不会回溯重建或猜测接入前的历史决策。它把当前源码 commit 记录为 **Adoption Boundary**，把共同目标和明确引用的需求记录为 **Common Ground**，保证后续开发可以从真实、统一的起点继续。

完整产品将在这个基础上加入分支路线、PR Context Capsule、签名证据和共享图谱投影。

Phase 1B 现在会把结构化生命周期事件和语义事件写入每个 worktree 对应 Git 目录下的 append-only journal。轻量的 Codex、Claude adapter 把原生生命周期/活动信号归一化到共享 event 与 journal contract；通用宿主通过本地 stdio MCP 端点和共享 Capture Kernel 显式记录语义条目。原生 Hooks 不保存完整 prompt transcript。

## 当前状态

| 能力 | 当前可用 | 规划中 |
| --- | :---: | :---: |
| 只读检查已有源码仓库 | ✓ | |
| Common Ground 草案和人类明确批准 | ✓ | |
| 使用普通 Git 分支的独立 Context Repository | ✓ | |
| Canonical JSON 与 SHA-256 内容标识 | ✓ | |
| 完整性验证和非零失败退出码 | ✓ | |
| 项目本地 Codex 与 Claude 捕获 Adapter | ✓ | |
| Generic MCP stdio 捕获端点 | ✓ | |
| 历史决策回填 | 明确排除 | |
| 原生 Agent 与 Subagent 生命周期捕获 | ✓ | |
| 显式结构化 Agent Decision 与备选方案 | ✓ | |
| 本地 Worktree 与 Agent 实时 Dock | ✓ | |
| 无需手动启动的 Codex MCP App 包 | ✓ | |
| 分支路线、PR Context Capsule 和 Merge Gate | | ✓ |
| 测试、构建和发布签名证明 | | ✓ |
| 交互式力导向拓扑 Viewer | | ✓ |

Capture Grade 根据运行时可验证的能力计算，而不是根据 adapter 名称或配置中的字面量计算。Codex Hooks、Claude Hooks 与 Generic MCP 当前的有效等级均为 **D**，即使配置完全匹配也是如此，因为 Phase 1B 尚不能观测 mutation 状态、建立 Evidence 与 mutation 的关联或映射到 commit。配置状态与有效激活状态会分开报告。

## 快速开始

### 1. 构建 DevMap

需要 Rust 1.96.1 或更高版本。

```bash
git clone https://github.com/DylanZhangzzz/DevMap.git
cd DevMap
cargo build --release
```

Unix 类系统的可执行文件是 `target/release/devmap`，Windows 下是 `target/release/devmap.exe`。

### 2. 创建 Common Ground 草案

源码仓库和 Context Repository 必须使用不同目录，Context Repository 不能位于源码仓库内部。

```bash
./target/release/devmap init \
  --source /work/payment-service \
  --context /work/payment-service-context \
  --goal "从当前 main commit 开始接入 DevMap" \
  --requirement "docs/requirements.md#payment-safety"
```

可选的 `#payment-safety` 片段会选择唯一匹配的 Markdown 标题。DevMap 只读取你明确指定的需求文档，不会扫描项目并猜测历史理由。

命令会输出：

- 作为 Adoption Boundary 的源码 commit；
- 接入时源码工作树是否存在未提交修改；
- Canonical 草案哈希；
- 完整的批准命令。

### 3. 评审并批准

先检查 Context Repository 中的 `bootstrap/common-ground-draft.json`，然后填写批准人的身份：

```bash
./target/release/devmap common-ground approve \
  --context /work/payment-service-context \
  --actor "Dylan"
```

批准会生成不可变的 Common Ground 与 Approval 对象，将 Context `main` fast-forward，并删除 `bootstrap/initial`。批准后再次运行 `init` 会被拒绝；未来变更必须明确 supersede 旧上下文，不能直接覆盖。

### 4. 验证完整性

```bash
./target/release/devmap status --context /work/payment-service-context
./target/release/devmap status --context /work/payment-service-context --json
```

`status` 会独立重算哈希，并验证对象 ID、Manifest 引用、Approval 绑定、Adoption Boundary、仓库状态和禁止出现的自定义 refs。证据缺失或被修改时，命令会输出 invalid 报告并返回非零退出码。

### 5. 启用并操作本地捕获

先预览将要发生的精确项目本地变更。需要时把宿主替换为 `claude` 或 `generic-mcp`：

```bash
./target/release/devmap adapter plan --source /work/payment-service --host codex
```

安装前检查输出的 bindings、目标路径和 `plan_digest`。digest 对精确的宿主、操作、source identity、原字节/文件 identity 和预期结果进行授权；期间发生任何修改都必须重新生成 plan。原生安装只会修改 `.codex/hooks.json` 或 `.claude/settings.json`；Generic MCP 只会修改 `.devmap/mcp.json`。

```bash
./target/release/devmap adapter install --source /work/payment-service --host codex \
  --plan-digest 'sha256-<reviewed-digest>'
./target/release/devmap adapter verify --source /work/payment-service --host codex
```

安装后检查生成的配置，并在开始捕获 session 前完成宿主要求的 trust 或 review 步骤。DevMap 不会绕过宿主的信任控制。`adapter verify` 会把配置漂移与有效激活分开报告；当 executable reachability、原生宿主 trust 或 managed-policy permission、Generic MCP 宿主注册状态无法验证时，会明确列出未解决的激活原因。不带 `--host` 时会同时验证 Codex、Claude 和 Generic MCP。

对于 Generic MCP 宿主，需要检查并注册 `.devmap/mcp.json` descriptor。它通过 stdio 启动 `devmap mcp --source .`。现代 discovery 只公布 `2026-07-28`；legacy `2025-11-25` 仅保留在 initialize 协商中，其他 legacy 版本会得到成功的受支持版本 counteroffer。

Codex Hooks、Claude Hooks 和 Generic MCP 当前的有效 Capture Grade 都是 D。原生 Hooks 只提供有界的生命周期与活动信号：`Stop` 表示一次 turn 完成，只有 `SessionEnd` 表示 session 完成。可写工具只产生工具活动和 `mutation_unverified` gap，绝不猜测 mutation。显式 Requirement、Decision 和 Evidence 记录统一通过 MCP/Capture Kernel surface 写入。只有未来可在运行时验证 mutation、Evidence 关联与 commit mapping 后，才可能提升到 Grade A。

Capture journal 按 worktree 保存在：

```text
<git rev-parse --git-dir>/devmap/sessions/<session-id>/events.ndjson
```

这些 append-only 本地证据不会被暂存到源码仓库。要仅移除 DevMap 所有的 bindings 或精确匹配的 Generic descriptor，必须先检查 removal plan 并传入独立 digest：

```bash
./target/release/devmap adapter plan --source /work/payment-service --host codex --action uninstall
./target/release/devmap adapter uninstall --source /work/payment-service --host codex \
  --plan-digest 'sha256-<reviewed-removal-digest>'
```

Phase 1B 只观察和记录；它**不会**创建或切换 branch/worktree，不会 stage、commit、stash、配置 remote 或 push。源码 Git 工作流管理将在后续阶段实现。

### 6. 在 Codex 中打开 Live Worktree Dock

一次性准备包括安装可执行文件，并在 Codex 中启用仓库内的 `plugins/devmap` 插件包：

```bash
cargo install --path .
```

插件安装并启用后，对 Codex 说“打开 DevMap Worktree Dock”即可。Codex 会以宿主管理的 STDIO 进程启动 `devmap mcp`，并在侧边面板打开 MCP App。正常使用不需要手动启动本地 HTTP server，也不需要另开 `devmap view --live` 进程。

Dock 的范围有意限制在本机：它显示与当前仓库共用 Git common directory 的 worktree，以及已启用 adapter 产生的 Presence。安装只需一次；运行时的启动和退出跟随 Codex task。项目 trust 设置或受管 MCP policy 可能禁用插件，此时 DevMap 必须如实说明，而不能声称 Dock 已激活。

对于不支持 MCP Apps 的环境，`devmap view --live --source PATH` 是可选、临时的 Browser fallback。它只绑定 loopback，并随命令退出；Codex 插件路径不依赖它。

## 核心语义

| 概念 | 含义 |
| --- | --- |
| Common Ground | 接入时经过明确评审的目标、源码边界与需求上下文 |
| Adoption Boundary | DevMap 从哪个精确源码 commit 之后开始保证证据链完整 |
| Requirement Trace | 对人类或权威文档要求的如实引用 |
| Agent Decision | Agent 自主选择的一条有意义路线，并包含 basis、alternatives、rationale、authority、scope 与 revisit trigger |
| Authority | 判断 Agent 是否有权作出该决定的策略 |
| Evidence | 与相关代码和 Claim 绑定的测试、构建、评审或证明 |
| Supersession | 明确表示新决定替代旧决定的关系 |

Agent 按照人类要求执行时，**不会**创建 Agent Decision。Phase 1B Kernel 仅在 Agent 面对多个有意义方向自主选择且发出显式结构化调用时记录 Decision；观察到 mutation 不会凭空生成 Decision。

## 存储模型

除了用户明确选择的项目本地 adapter 文件（`.codex/hooks.json`、`.claude/settings.json` 或 `.devmap/mcp.json`），Phase 1B 不会写入源码 worktree，也不会修改源码 Git 状态。Capture journal 位于每个 worktree 对应的 Git 目录下。Phase 1A 的 Canonical Context 保存在独立仓库中，使用普通分支和 commit：

```text
payment-service-context/
├── .devmap-context.json
├── objects/
│   ├── approval/<sha256>.json
│   └── common-ground/<sha256>.json
├── manifests/common-ground.json
└── state/current.json
```

DevMap 只暂存自己的路径；发现意外文件时 Bot commit 会停止。Canonical Object 路径不能越出 Context Repository，并且系统不会创建 custom refs 或 Git Notes。

因为 Context `main` 是普通 Git，现有仓库权限、评审、备份、克隆和审计工具可以继续跨平台使用。

## 给 AI Agent 的读取规则

读取或扩展 DevMap 项目时遵循以下不变量：

1. 不得根据 Adoption Boundary 之前的历史推测决策、备选方案或理由。
2. Requirement Trace 表示人类意图，不是 Agent Decision。
3. 先读取 `state/current.json`、对应 Manifest 和相关 Canonical Objects，再决定是否加载更大上下文。
4. 信任对象前必须验证内容 ID 和哈希。
5. Capture Grade 表示基于能力的实际观测覆盖。Phase 1B 原生 Hooks 与 Generic MCP 即使配置精确也都是 Grade D；配置、激活与能力等级是三个独立事实。更高等级要求可在运行时验证 mutation、Evidence 关联与 commit mapping。
6. 不得覆盖 Canonical Objects；未来变更必须明确建立 supersession。

## 路线图

- [x] **Phase 1A — 真实接入基础：** Common Ground、Adoption Boundary、Context Repository、Canonical ID 和完整性验证。
- [x] **Phase 1B — Capture Kernel：** 宿主中立协议、薄 Agent Adapter、Capability Handshake、SessionStart/SubagentStart 传播和 Capture Gap 报告。
- [ ] **Phase 2 — PR 证据链：** Route Branch、Agent Decision、Claim、PR Context Capsule、Merge Gate、Context Bot 接入和签名证明。
- [ ] **Phase 3 — 开发拓扑：** W3C PROV 投影、本地只读 Viewer、语义缩放、共享 Graph State、PM 过滤器和交互式证据路径。

## 开发

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
```

完整系统契约请阅读[产品需求文档](docs/ai-development-map-requirements.md)。[Phase 1A 实施计划](docs/superpowers/plans/2026-08-26-devmap-phase-1a-core.md)与 [Phase 1B 实施计划](docs/superpowers/plans/2026-08-27-devmap-phase-1b-native-capture.md)记录了当前已交付基础与明确延期边界。
