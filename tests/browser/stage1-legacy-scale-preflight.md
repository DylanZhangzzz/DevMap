# Pure legacy scale input and public-map audit

This is preparation for Stage 1 A/A calibration. No formal candidate population or calibrated tolerance has been published.

## Fresh input

The existing ignored `generate_disposable_legacy_scale_corpus` entry generated a new repository under the checkout's verification directory: `scale-legacy-jI7MaE`. The compiled test executable SHA-256 is AA39FB1EA196615E61F76A35AE0A5552BD0FEC345C9F64555D63F31F4E72A18A, from the current full regression run. It ran inside the default strict Windows Job wrapper; root 32244 exited 0, native Job accounting confirmed empty, and the test passed in 30.28 seconds.

Read-only finalization verified 20 real worktree paths, 100 journal files with 1,000 JSON records each, 100,000 total records, 93,086,400 journal bytes and no SQLite database. The 624-entry legacy inventory digest is `0435cedeaf4bf4744ead9210df0567531cd88a060b47a2cb56c971bf7e558436`. This synthetic corpus was generated through domain APIs, not by a real host. It is a fresh pure-legacy baseline input; it has not yet been migrated or proved matched to SQL.

Generation evidence is in `target/verification/stage1-legacy-generation-F0uVDu`. Its original `report.json` remains failed: the postprocessor incorrectly expected `SCALE_FIXTURE` at the start of a line, while Rust printed it after the test name. `finalized.json` records the subsequent read-only validation. No regeneration, deletion or rewrite of the failed receipt occurred. The exact local runner is retained as `target/verification/stage1-create-legacy-scale.cjs`; its parser remains historical failed evidence, not a reusable passing generator.

## Full-map comparison rules

`process-performance.cjs` supports opt-in `DEVMAP_BENCHMARK_VERIFY_MODEL=1`. It records full-model fingerprints after each measured interval, plus the initial full response; failures retain up to four mismatching responses. Hashing is outside each request duration but affects pacing, so both sides must use the same audit work in any future comparison. This does not measure browser-ready timing.

Default `strict` mode ignores only top-level `generated_at`. Every identity, business revision, array order, nested timestamp and freshness field otherwise participates. `fresh-git-observation` mode additionally **requires** each non-null workspace Git observation time to equal the current generation time before normalizing that exact relationship; null remains distinct. Older/cached observations fail this mode and require a separate freshness audit, rather than silent timestamp removal.

Scale runs LMCvKz and kk063Z failed strict comparison because Git observation times advanced with generation time. The kk063Z stored diff contains exactly these 21 timestamp paths. A subsequent ybhR4T run failed the fresh-observation policy on `observation_revision` advancing from 2 to 3. Source `finalize_revisions` increments this observation counter on every refresh, independently of content revision. All three failed runs preserved legacy inventory and confirmed empty Jobs; none is relabeled passing.

The additional `legacy-refresh` mode is executable-hash restricted to frozen A1CF. It checks that every client's first observation counter equals the same initial counter and that subsequent maps advance exactly once; only after verification does it normalize that counter for content comparison. Measured rows retain the actual counter. Git timestamps still must equal generation time. This baseline-specific rule is not a candidate compatibility exemption. Candidate shared observation/caching semantics must be audited explicitly before formal comparison.

Three Node tests cover strict timestamp sensitivity, rejection of stale timestamps, preservation of null/unknown observations, business and identity changes, task timestamps, revisions, array order, and nonmutation of input. The actual scale preflight exercises four clients through the frozen public MCP endpoint. It remains a small preflight, not adequate A/A samples or performance acceptance.

Final actual baseline run `stage1-scale-public-T5pEW1` passed: one cold request, one warmup and three measured reads per client, four clients, code 0, strict Job empty and unchanged legacy inventory with DB still absent. Counters were 3, 4, 5 for each measured client sequence. The verified content fingerprint was `2a121e3e0ab7789dcc1f121c100fe1785df85509b3e1e01559ff5b38712d38eb`. Cold was 7212.253 ms; each client's maximum of three warm observations was 4585.445 / 4567.700 / 4589.877 / 4598.172 ms. Maximum full MCP response was 207,094 bytes. These are small exploratory full-map observations, not summary-size violations, representative p95 estimates, A/A calibration, or evidence for widening the existing tolerance caps.

The next step is to complete the baseline-only calibration controller with independent epochs and a fixed sample schedule. Then migrate this same repository through the supported migration path and compare exact inputs and public output semantics. Preserve the pure-legacy reports and immutable sources. Do not create a second path with silently remapped repository/worktree identities and call it an exact comparison.
