# Registered legacy scale input with tracked change probe

Commit bfd4481 adds an opt-in `DEVMAP_SCALE_TRACKED_PROBE=1` to the explicit
ignored legacy generator. The probe is written and committed before linked
worktrees, journals and presence records are generated. The original no-probe
generation path remains available. No production Rust source or release binary
changed.

The formatted test target compiled successfully in 13.01 seconds. Its retained
executable is `target/verification/change-generator-b6188ebb/performance_fixture.exe`,
SHA-256 `8C84E04CCB2669CD85D4FE1DD3A480BE16134E201605E1DBEAA6884BF63D3B66`.
The matching fixture source is retained beside it; build messages are in
`target/verification/change-fixture-formatted-build.jsonl`. The earlier pre-format
build and snapshot were retained, not substituted for this generator.

`create-legacy-change-scale.cjs` pins that executable, checks allocation space,
uses an isolated temp/state environment, owns generation through a strict Windows
Job, and registers only a newly allocated corpus after successful completion.
It verifies 100 journal files with 1000 records each and that every record's
context HEAD matches its worktree HEAD. It records physical identities, clean Git
states, probe bytes/hash, manifest hash and full legacy inventory before any
change measurement. It never migrates the corpus or enables host hooks.

Generation run `legacy-change-generation-CSuFvK`, session 11445, exited 0.
The Rust generator passed in 35.22 seconds, and the strict Job was empty. The
new corpus is `scale-legacy-h7vZyC`: 20 worktrees, 100 sessions, 100000 records,
92986400 event bytes, no database. Its legacy inventory SHA is
`a92aacf2930a4db9858b660d279f8ba6a0cca36bf39b82b822d1524d32ea7bbd`.
Initial HEAD is `522930ab94def05b7b2bb03775604e0a17d238de`; the main tracked
`devmap-change-probe.txt` SHA is
`83d8589654d1312bc1cd93f7e9319463aff7fd2b8ed2975c2055485a4cd416f0`.
The generator report and `legacy-inventory.json` are the registration receipt.

## Actual public full-map scale preflight

`full-map-change-preflight.cjs --owned` now accepts optional absolute
`DEVMAP_CHANGE_SCALE_RECEIPT` pointing to the above completed report. It checks
the registered input and operates the existing probe, without adding a hook or
another session. With no receipt it still creates the independent small fixture.
The schedule remains two warmups and four measured changes across four old A1CF
MCP clients; it is explicitly not the 100-change A/A population.

Run `full-map-change-Hs40L8`, session 86793, exited 0 and its strict Job confirmed
empty. All 24 client/change observations passed exact whole-model delta checks;
the 16 measured observations were 5925.1–6408.1 ms. These are old-only exploratory
full-map visibility times, not summary latency, a calibrated p95, or acceptance.
Every proxy exited normally on EOF. Afterward all 20 worktrees were clean with
unchanged HEADs, the probe was restored, the complete registered legacy inventory
matched and no DB existed.

A separate read-only audit of all 32 initial/change/final response files confirmed
20 workspaces and 100 distinct expected sessions in every response, correct
session-to-worktree assignment, correct HEADs and `truncated=false`. The result is
`full-map-change-Hs40L8/scale-model-audit.json`. That audit also verified the
original cold/warm corpus jI7MaE's complete legacy inventory remained unchanged.
Four model/predicate/population controls passed after the native run; rustfmt
verification passed.

This input has a different initial Git tree and repository identity from jI7MaE.
Do not mix the two as one paired population or reuse old cold/warm calibration
numbers as calibration of the new corpus. The exact input for formal old/new
comparison must be registered consistently for both sides, with a new supported
migration and preserved sources. Still open: full 100-change A/A sampling and
analysis, cold/warm noise calibration, browser feedback/ready calibration and the
committed formal contract. No tolerance was frozen or widened.
