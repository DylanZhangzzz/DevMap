# DevMap npm and GitHub release design

Approved direction: retain Rust, distribute prebuilt binaries through npm, and automate packaging on GitHub. The user's 2026-09-06 request authorizes implementation of that direction.

## Scope and architecture
A single npm package bundles five prebuilt binaries: Windows x64, macOS x64/arm64, Linux glibc x64/arm64. A dependency-free Node launcher selects the bundled binary and preserves arguments, cwd, environment, stdio and exit status. No Rust installation, postinstall script or runtime download is required. Git remains required. Node >=22 is required for npx.
Bundling all platforms costs download size, but makes first publication and installation atomic with one npm package and one trusted publisher. Platform optional dependencies can follow if measured package size warrants six independently published packages.

GitHub Actions validates changes, compiles on native hosted runners, smoke-tests a real npm tarball on each platform, and assembles archives, checksums and the full npm tarball. Manual runs build artifacts without publication. v* tags must match Cargo.toml and produce a GitHub Release. npm publication is separately enabled using a repository variable after package ownership and trusted publishing are configured. No registry name is represented as already published.

## Boundaries
Keep the existing locally installed plugin working until a registry package actually exists. Document the npx MCP configuration separately. Do not replace the installed runtime, push a release tag, or claim hosted builds passed without evidence.
Do not rewrite the Rust core or modify source-repository semantics.

## Verification
Node tests cover platform rejection, omitted binaries, safe version/tag handling and staging completeness. Pack/install smoke tests use a real built DevMap executable, a temporary Git repository with spaces, failing CLI arguments, and MCP initialize/tools/list over stdio. Native-runner smoke tests are required before a tag can publish. Checksums cover downloadable assets. CI validates Rust formatting, clippy and tests.
