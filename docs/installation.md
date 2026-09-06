# Install DevMap without Rust

DevMap can run from a prebuilt npm tarball or a native archive. Building from source remains available for contributors.

## Release availability

This repository contains the packaging pipeline. It does not imply that a package has already been published to npm. Check the [GitHub Releases page](https://github.com/DylanZhangzzz/DevMap/releases) for real assets and the maintainer's selected npm name. `YOUR_PACKAGE` and `VERSION` below are placeholders; `devmap-local-preview` is a build-only fallback name, not an advertised registry package.

## npm / npx

Requirements: Git on PATH, Node.js 22+ with npm. Rust is not needed.

After the maintainer publishes a package, run this from the Git repository to inspect, replacing the placeholders:

```sh
npx --yes YOUR_PACKAGE@VERSION view --live --source .
```

Open the private loopback URL it prints. Keep the terminal running. To read Git facts:

```sh
npx --yes YOUR_PACKAGE@VERSION agents --source . --json
```

The npm package contains all five native executables. The launcher selects the current OS/CPU. There are no npm lifecycle/install scripts, no runtime executable downloads and no Rust compilation. This first release favors one atomic package over multiple platform packages; npm downloads all included platforms.

### Before first npm publication

Download the `.tgz` npm asset from a completed GitHub build or Release. Use an absolute path, especially when your repository and download directory differ:

```sh
npm exec --yes --package="/absolute/path/to/devmap-package.tgz" -- devmap view --live --source .
```

On Windows a quoted path such as `C:/Downloads/devmap-package.tgz` works. The exact filename appears in the build artifact; do not substitute a native `.tar.gz` archive here.

### MCP configuration

Replace the package name/version and repository path:

Run `npx --yes YOUR_PACKAGE@VERSION --version` once in a terminal first to cache the package before a host starts its MCP connection with a short startup timeout.

```json
{
  "mcpServers": {
    "devmap": {
      "command": "npx",
      "args": ["--yes", "YOUR_PACKAGE@VERSION", "mcp", "--source", "/absolute/path/to/repository"]
    }
  }
}
```

Use your host's supported npx command resolution on Windows. A host that cannot launch `npx.cmd` can use `node` as the command and the absolute path to npm's `npm-cli.js` followed by `exec --yes --package=YOUR_PACKAGE@VERSION -- devmap mcp --source ...` as individual arguments. The launcher itself never invokes a shell.

An MCP entry runs the tools; it does not install a Skill. The bundled [Skill](../plugins/devmap/skills/live-worktree-dock/SKILL.md) supplies Codex-specific inventory and navigation instructions. The existing plugin configuration continues using `devmap mcp` so existing installations do not break before the npm package is published. After publication, `npm install --global YOUR_PACKAGE@VERSION` provides a `devmap` command for that existing configuration.

A standalone map shows Git worktrees. Complete Agent task observations and task navigation require host integration; npx alone does not provide that data.

## Native download: no Node or Rust

Download the `devmap-VERSION-TARGET.tar.gz` asset for your system, compare its SHA-256 with `SHA256SUMS`, and extract it. For example:

```sh
tar -xzf devmap-VERSION-TARGET.tar.gz
./devmap view --live --source /path/to/repository
```

On Windows use `.\devmap.exe` after extraction. Keep the executable in a stable directory and add that directory to PATH to use the existing plugin.

| System | Release target | Baseline |
| --- | --- | --- |
| Windows x64 | x86_64-pc-windows-msvc | Built/tested on Windows Server 2022 |
| macOS Intel | x86_64-apple-darwin | Built/tested on macOS 15 |
| macOS Apple Silicon | aarch64-apple-darwin | Built/tested on macOS 14 |
| Linux x64 | x86_64-unknown-linux-gnu | Built/tested on Ubuntu 22.04, glibc 2.35 |
| Linux ARM64 | aarch64-unknown-linux-gnu | Built/tested on Ubuntu 22.04, glibc 2.35 |

These describe the configured CI baselines, not a claim that hosted builds have already passed. Older systems are not validated. Alpine/musl and Windows ARM64 are not in the matrix. macOS archives are not signed/notarized. Git is required for every installation method.

## Developers

```sh
git clone https://github.com/DylanZhangzzz/DevMap.git
cd DevMap
cargo install --locked --path .
```

Source builds require Rust 1.96+. See [release maintenance](releasing.md) for automation and local package verification.
