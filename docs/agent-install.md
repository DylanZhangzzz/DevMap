# Install DevMap with your Agent

Give your Codex Agent this request:

> Read https://github.com/DylanZhangzzz/DevMap/blob/main/docs/agent-install.md and install DevMap for Codex, including its Skill and MCP configuration. Verify the installation and tell me whether a new task is needed.

中文：

> 请阅读 https://github.com/DylanZhangzzz/DevMap/blob/main/docs/agent-install.md，帮我安装并配置 DevMap，包括 Codex 插件、Skill 和 MCP。完成后验证安装，告诉我是否需要新建任务。

## Instructions for the installing Agent

These steps configure the current user's Codex installation. npm alone installs the executable; the Codex plugin bundles the Skill and MCP configuration. No Rust build, npm account, global npm install, or manual configuration file editing is needed.

1. Check `git --version`, `node --version`, `npm --version`, and `codex plugin add --help`. Node.js 22+ is required. Supported systems: Windows x64, macOS Intel/Apple Silicon, Linux glibc 2.35+ x64/ARM64. If a prerequisite is missing, report it and use the user's authorized installation method; do not claim success. A Codex host without the plugin CLI needs an updated client before this route works.
2. Inspect `codex plugin list --json` and `codex mcp list --json`. If DevMap is already installed from another marketplace or as a standalone MCP server, report the existing entry and avoid enabling duplicate copies. Do not remove unrelated settings or migrate an existing installation without authorization.
3. Cache and verify the published executable:

   ```sh
   npx --yes devmap-cli@0.1.1 --version
   ```

4. Register the public repository marketplace and install the plugin:

   ```sh
   codex plugin marketplace add DylanZhangzzz/DevMap --ref main
   codex plugin add devmap@devmap-marketplace --json
   codex plugin list --json
   ```

   If this marketplace is already registered, refresh it with `codex plugin marketplace upgrade devmap-marketplace` before installing. The plugin includes `live-worktree-dock` and starts `npx --yes devmap-cli@0.1.1 mcp`. Do not add a second standalone MCP entry. Node/npm must also be on the desktop host's PATH; restart the host after installing Node if necessary.

5. Confirm the plugin is installed and enabled with no reported load errors. Confirm its installed manifest includes `skills` and `mcpServers`, and that the referenced files exist. `codex mcp list` alone is not proof that plugin-provided tools loaded. In the user's intended Git repository, run:

   ```sh
   npx --yes devmap-cli@0.1.1 agents --source . --json
   ```

6. Report installation separately from runtime readiness. Start a new Codex task in the intended repository to load the new Skill/tools; restart the app if they are still absent. In that task ask **“Open DevMap in the right sidebar” / “在右侧栏打开 DevMap”**. Follow the bundled Skill, check that `devmap_open_map` is callable, and verify the map opens. Complete task inventory/navigation depends on the host exposing its task tools. Do not invent Agent observations or claim a map rendered just because installation succeeded.

The plugin uses the host-provided working directory rather than pinning every project to one repository. It does not automatically install optional capture adapters or grant write permissions. Those remain separate, explicitly requested operations.

See [installation options](installation.md) for native binaries and generic MCP hosts, and [OpenAI's plugin documentation](https://developers.openai.com/plugins/build/plugins) for marketplace behavior.
