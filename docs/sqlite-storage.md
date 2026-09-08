# Repository SQLite storage

This guide describes the `codex/devmap-sqlite-compatible` candidate. It is not
an announcement that an installed plugin or published package has changed.
The [acceptance record](audits/2026-09-08-sqlite-compatibility.md) tracks the
remaining startup, performance and host checks.

## What is shared

Linked Git worktrees share one authoritative database at
`<git-common-dir>/devmap/devmap.db`. Resolve the common directory with
`git rev-parse --path-format=absolute --git-common-dir`; a linked worktree's
`.git` file is not the database directory.

The store contains accepted events, route revisions and original route starts,
presence, task associations and their observation watermarks. Existing Context
Git repositories and objects remain in their original locations. They are not
deleted or silently imported into SQLite.

The MCP process and explicit browser entry use a repository-scoped shared core.
The existing browser assets and map schema remain compatible. SQLite is bundled
into the native binary; no separate database server is required. The core exits
after its clients and idle lease expire and can be started again on demand.

One database does not mean that deleting every adjacent file is safe. WAL/SHM
files, locks and activation provenance support consistency and recovery. Keep
the complete directory and retained backups when investigating a failure.

## Inspect and verify

These commands do not activate a missing database:

```sh
devmap storage inspect --source /path/to/repository
devmap storage verify --source /path/to/repository
```

`inspect` reports the selected backend and metadata. `verify` requires an
existing database and checks its integrity, domain records and migration
provenance. A successful inspection is not a substitute for verification.

## Controlled migration of existing legacy data

Existing legacy history is preserved by default. Before an operator-controlled
cutover, stop all writers using the old implementation, including host hooks.
The candidate's lock coordinates candidate writers; an older binary does not
participate in that lock.

Choose a new backup directory whose parent already exists outside Git
administration:

```sh
devmap storage migrate --source /path/to/repository --backup-dir /durable/backups/repository-activation
devmap storage verify --source /path/to/repository
```

Migration freezes and hashes the original files, validates the frozen input,
imports a shadow store, compares domain and map semantics, and records durable
activation provenance before selecting SQLite. The originals and frozen copy
remain available. A retry must refer to the same retained snapshot; a different
snapshot cannot reset accepted SQL history.

For a repository with no legacy artifacts, the first valid shared write sets up
SQLite automatically. Merely viewing a map does not create a database or start
migration. Existing legacy artifacts, including empty session directories, keep
the legacy backend until controlled migration. Invalid input is rejected before
setup. Interrupted or unowned shadow state requires explicit recovery.

On Windows, the conventional user profile layout stores fresh-activation backups
under `%USERPROFILE%/.devmap-state/DevMap/backups`; a redirected `LOCALAPPDATA`
is respected and validated. Unix uses `XDG_STATE_HOME/devmap/backups`, or
`$HOME/.local/state/devmap/backups`. Each repository has a separate directory
and each attempt has a unique, retained ownership record and frozen snapshot.
The candidate requires trusted local parent directories; it does not weaken
permissions on shared writable paths to make initialization succeed. Windows
default path selection has been checked read-only; this branch has not activated
any real user repository or written its default backup directory.

## Consistent database backup

Use a new destination file in an existing directory outside Git administration:

```sh
devmap storage backup --source /path/to/repository --destination /durable/backups/repository.db
```

The command uses SQLite's consistent backup interface and validates the copy.
Copying only a live main `.db` file can omit committed records still in its WAL.
Existing destinations are refused so a previous backup is not overwritten.

## Recovery boundary

Retain the database, sidecars, activation record, original legacy files and
frozen snapshot when a check fails. Missing data after an activation fence,
unknown schema, source drift or a mismatched snapshot require diagnosis; the
candidate does not delete the database and revive an older log automatically.

A late old writer is detected as legacy source drift. Both histories remain
available for forward recovery. Once SQL has accepted new records, reverting
to the old frozen snapshot would lose them. No general lossless reverse-export
or automatic downgrade is provided by this candidate.

The maintenance CLI does not offer an in-place restore command. Restore
validation is performed on isolated copies in tests; replacing a live database
while processes retain SQLite handles is outside this workflow.
