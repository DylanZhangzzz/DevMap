# DevMap

A local Git worktree map for humans and AI agents. See real commit history, workspace state, related tasks and recorded delivery plans together.

This package includes prebuilt DevMap binaries for Windows x64, macOS Intel/Apple Silicon, and Linux glibc 2.35+ x64/arm64. It has no install scripts and downloads no executable at runtime. Rust is not required. Node.js 22+ and Git must be installed. Linux 2.35 is the supported glibc baseline, not a measured minimum ELF symbol requirement.

Run the package with npx, followed by `view --live --source .` to show the map, or `mcp --source /absolute/path/to/repository` to start the MCP stdio server. Use an explicit version for reproducibility. The published package name is in package.json.

The viewer prints a private loopback URL; open it in your browser. Keep the process running. Git viewing is standalone; agent task observations depend on host integration. DevMap does not execute merges, pushes or tests.

Linux Alpine/musl and Windows ARM64 are not included in this release matrix. Build from source on unsupported systems. macOS artifacts are not signed or notarized.

Source, installation and release documentation: https://github.com/DylanZhangzzz/DevMap
