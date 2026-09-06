# Install DevMap without Rust

DevMap can run from a prebuilt npm tarball or a native archive. Building from source remains available for contributors.

## Release availability

Use the prebuilt package from [release v0.1.0](https://github.com/DylanZhangzzz/DevMap/releases/tag/v0.1.0). Its stable filename is `devmap-0.1.0.tgz`, independent of the eventual npm account or scope. GitHub installation does not require npm registry publication or an npm account. The registry package is `devmap-cli`; the examples pin version 0.1.0.

## Agent installation for Codex

Give your Agent the [installation guide](agent-install.md). It installs the public Codex plugin with both the Skill and MCP configuration. npm by itself does not configure Codex.

## npm / npx

Requirements: Git on PATH, Node.js 22+ with npm. Rust is not needed.

Run this from the Git repository to inspect:

```sh
npx --yes devmap-cli@0.1.0 view --live --source .
```

Open the private loopback URL it prints. Keep the terminal running. To read Git facts:

```sh
npx --yes devmap-cli@0.1.0 agents --source . --json
```

The npm package contains all five native executables. The launcher selects the current OS/CPU. There are no npm lifecycle/install scripts, no runtime executable downloads and no Rust compilation. This first release favors one atomic package over multiple platform packages; npm downloads all included platforms.

### Direct GitHub installation

The same fixed-version package can be installed directly from GitHub without using the npm registry:

```sh
npx --yes https://github.com/DylanZhangzzz/DevMap/releases/download/v0.1.0/devmap-0.1.0.tgz view --live --source .
```

### Install a downloaded package

You can also download `devmap-0.1.0.tgz` from GitHub and use its absolute local path, especially when your repository and download directory differ:

```sh
npm exec --yes --package="/absolute/path/to/devmap-0.1.0.tgz" -- devmap view --live --source .
```

On Windows a quoted path such as `C:/Downloads/devmap-0.1.0.tgz` works. Do not substitute a native `.tar.gz` archive here.

### MCP configuration

Replace the repository path:

Run the Quick start command with `--version` instead of `view --live --source .` once in a terminal first to cache the package before a host starts its MCP connection with a short startup timeout.

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

Use your host's supported npx command resolution on Windows. A host that cannot launch `npx.cmd` can use `node` as the command and the absolute path to npm's `npm-cli.js` followed by `exec --yes --package=devmap-cli@0.1.0 -- devmap mcp --source ...` as individual arguments. The launcher itself never invokes a shell.

An MCP entry runs the tools; it does not install a Skill. For Codex use the [plugin installation guide](agent-install.md), which installs both. The public plugin starts `npx --yes devmap-cli@0.1.0 mcp` and does not require a globally installed `devmap` command.

A standalone map shows Git worktrees. Complete Agent task observations and task navigation require host integration; npx alone does not provide that data.

## Native download: no Node or Rust

Download the `devmap-VERSION-TARGET.tar.gz` asset for your system, compare its SHA-256 with `SHA256SUMS`, and extract it. For example:

```sh
tar -xzf devmap-VERSION-TARGET.tar.gz
./devmap view --live --source /path/to/repository
```

On Windows use `.\devmap.exe` after extraction. Keep the executable in a stable directory. For a Node-free MCP setup, configure your host to launch its absolute path with `mcp`; the public Codex plugin uses npx and requires Node.

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
