# Build and publish DevMap releases

## What is automated

`.github/workflows/release.yml` runs on pushes to main/codex branches, pull requests, manual dispatch and `v*` tag pushes. Branch pushes, pull requests and manual runs only produce artifacts. Tag pushes also create a GitHub Release after checks and all five native package smoke tests pass.

The workflow:
1. Runs Rust formatting, clippy and tests plus Node packaging tests.
2. Compiles with Rust 1.96.1 and Cargo.lock on five native hosted runners.
3. Packs and installs a real npm tarball on each runner with install scripts disabled; verifies CLI, Git paths, exit status and MCP stdio.
4. Requires all five binaries before assembling the npm package.
5. Uploads five native archives, one npm tarball and SHA256SUMS as `release-assets`.
6. Publishes GitHub assets on version tags. npm publishing is a separate opt-in job.

Cargo.toml is the only release version source. A tag that differs from its version fails. Prerelease versions use npm's `next` tag and GitHub's prerelease flag. Existing releases are not overwritten; rerun failed jobs instead of replacing successful public release assets.

## First build: no npm account required

Open Actions > Package and release > Run workflow, push a main/codex branch, or open a pull request. Download the `release-assets` artifact after success. The default package metadata name is `devmap-cli`; this does not itself publish or reserve that npm name. The archive has the stable filename `devmap-VERSION.tgz` even if the npm name changes. Tag releases make that file publicly downloadable and installable by URL without an npm account.

## Enable npm publication once

The initial package `devmap-cli@0.1.0` was published to npm under `dylan_zhang` on 2026-09-06. The steps below configure automatic publication of future versions; the first public package alone does not enable OIDC publishing.

Set repository variable `DEVMAP_NPM_PACKAGE=devmap-cli`. If using a different package name in a fork, establish ownership and rebuild the tarball with that name. GitHub usernames do not prove npm ownership. A different npm name does not change the GitHub download URL.

For an initial package, publish that complete tarball from your authenticated npm account (`npm login`, then `npm publish PATH_TO_TARBALL --access public`). If using a prerelease, add `--tag next`. Do not publish the single-platform local smoke package.

In the npm package's trusted publishing settings authorize:
- GitHub owner: DylanZhangzzz
- Repository: DevMap
- Workflow: release.yml
- Environment: none (the workflow has no environment)
- Permission: direct npm publication if automatic public publishing is intended

Then set repository variable `DEVMAP_NPM_PUBLISH=true`. Subsequent new version tags publish using OIDC; no long-lived npm token is needed. The workflow uses Node 24, whose bundled npm satisfies the required npm 11.5.1+ / Node 22.14+ floor. Check the logged npm version if changing runners or Node versions.

The first version manually published is already used; the next automated publication must increment Cargo.toml and update Cargo.lock. Do not enable publishing before package ownership is established. Until this setup is complete, public GitHub downloads work independently.

Reference: [npm trusted publishing](https://docs.npmjs.com/trusted-publishers/).

## Cut a release

1. Update the version in Cargo.toml and refresh Cargo.lock with Cargo. Review both files.
2. Merge the validated change through the repository's normal review process.
3. Create and push the matching `vVERSION` tag on that commit.
4. Inspect every build, package smoke test, release asset and npm publication result. A green build is not proof that a skipped publication happened.
5. Update installation examples with the confirmed package name/version after the first public publication.

The workflow does not create version tags. Do not repoint a published tag to different code. It uses native runners listed in [GitHub's runner documentation](https://docs.github.com/en/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job).

## Local Windows verification

```powershell
cargo build --release --locked
node --test packaging/tests/*.test.cjs
node packaging/release.cjs native win32-x64 target/release/devmap.exe dist
node packaging/smoke.cjs dist/native
```

On another supported host use its platform key and native binary. Assembly requires artifacts from all five runners:

```sh
node packaging/release.cjs assemble dist YOUR_PACKAGE
```

Smoke packages contain one platform for testing only. Publish only the full `release-assets` tarball. Generated artifacts live under ignored `dist/`; no binaries enter Git.
