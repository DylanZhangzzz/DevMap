# DevMap Live Worktree Dock MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a compact live Dock that shows the current and other local Git worktrees and their instrumented Agents in a Codex MCP App without manual server startup, with an on-demand localhost Browser fallback.

**Architecture:** Phase 1B hooks and MCP calls update bounded, disposable Presence records under the Git common directory. A worktree scanner and Presence reducer produce one revisioned `DockReadModel`; the existing Rust MCP process exposes it over STDIO with an MCP Apps resource, while `devmap view --live` exposes the same model through a temporary loopback HTTP/SSE bridge. The two presentations share one embedded, dependency-free frontend and neither writes canonical Context data.

**Tech Stack:** Rust 1.96, Cargo, clap 4, serde/serde_json, sha2, time, fs2, `getrandom` 0.3, `tiny_http` 0.12, vanilla HTML/CSS/JavaScript, MCP Apps `ui/*` bridge, and the system Git executable. No Node.js runtime, React, database, daemon, custom Git refs, Git Notes, CDN, or private Codex UI API.

**Spec:** `docs/superpowers/specs/2026-09-02-devmap-live-worktree-dock-design.md`

## Global Constraints

- Execute from a new isolated worktree only after Phase 1B commit `e620cbf` is an ancestor of the implementation branch.
- Do not implement against the current Phase 1A-only source tree; the existing `src/events.rs`, `src/journal.rs`, `src/hook.rs`, `src/mcp.rs`, and adapter conformance tests are required inputs.
- Presence lives only at `<git-common-dir>/devmap/presence/v1/<session-id>.json`; it never enters source commits, Context Repository commits, attestations, or canonical graph objects.
- The same Project Graph revision and global topology layout must remain unchanged when Presence is absent, stale, or corrupt.
- Only explicit SessionEnd/`session_stopped` may produce `completed`; lease expiry produces `stale`, never `completed`.
- A host that cannot prove an Agent state reports `unknown` or reduced confidence; it never guesses.
- Every string, file, collection, HTTP response, and MCP response is bounded. The hard limits are 256 worktrees, 2,048 Presence records, 64 KiB per Presence record, and 1 MiB per MCP line.
- Presence and Dock output exclude raw prompts, commands, patches, tool arguments, tool results, file contents, and transcripts.
- The default Codex path is `devmap mcp` over host-managed STDIO and opens no TCP listener.
- The HTTP server starts only for Browser fallback or an explicit `devmap view --live`, binds `127.0.0.1` on a random port, requires an ephemeral token, and exposes no mutation endpoint.
- The frontend is embedded, makes no CDN or off-machine request, and uses capability detection rather than product-name branching.
- `work/` is user-owned and must never be modified, staged, or committed.
- Every behavior change begins with a failing automated test and ends with a focused commit.

## Scope boundary

This plan implements delivery steps 1–5 from the approved design. It keeps `route_id` and selection fields in the public model, but uses a `NoRoutes` provider until the Phase 1C route registry exists. Selecting a row updates portable MCP model context or Browser-local selection; connecting that selection to the full topology Viewer and evidence-neighborhood focus is a separate plan after the shared graph Viewer is available.

---

## Target file layout

```text
Cargo.toml
assets/
  dock.html                         # One self-contained MCP/Browser UI document
plugins/
  devmap/
    .codex-plugin/plugin.json       # Installable plugin manifest
    .mcp.json                       # Host-managed STDIO launch configuration
    skills/live-worktree-dock/
      SKILL.md                      # Teaches Agents when to render the Dock
src/
  cli.rs                            # agents and view --live surfaces; optional MCP source
  dock.rs                           # DockReadModel, reducer, revision service, route seam
  dock_asset.rs                     # Embedded UI resource and CSP metadata
  error.rs                          # Worktree, Presence, and Viewer errors
  events.rs                         # Existing event accessors used by Presence projection
  git.rs                            # Git common-dir discovery
  hook.rs                           # Non-authoritative Presence projection after capture
  journal.rs                        # Read-only bounded journal summaries
  lib.rs                            # CLI dispatch
  mcp.rs                            # Existing capture tools plus Dock data/render/resource APIs
  presence.rs                       # Presence schema, status semantics, atomic store
  viewer.rs                         # Temporary loopback HTTP/SSE Browser bridge
  worktrees.rs                      # Porcelain parser and stable Git-dir identities
tests/
  dock_mcp.rs
  dock_model.rs
  dock_plugin.rs
  dock_ui_contract.rs
  dock_viewer.rs
  live_dock_acceptance.rs
  presence_store.rs
  worktree_inventory.rs
  support/mod.rs
.superpowers/sdd/2026-09-02-devmap-live-worktree-dock-mvp/
  verification-report.md             # Manual UI and final-gate evidence
```

### Task 1: Discover stable repository and worktree identities

**Files:**

- Modify: `src/git.rs`
- Create: `src/worktrees.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/worktree_inventory.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**

- Consumes: Phase 1B `SourceGitInspector`, `SourceWorkspace`, `sha256_hex`, and linked-worktree fixtures.
- Produces: the exact `git_common_dir` field and `WorktreeScanner::scan` API used by Presence and Dock tasks.

```rust
pub struct SourceWorkspace {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub git_common_dir: PathBuf,
    pub branch: Option<String>,
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeDescriptor {
    pub worktree_id: String,
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub branch: Option<String>,
    pub head: String,
    pub is_current: bool,
    pub is_bare: bool,
    pub is_locked: bool,
    pub is_prunable: bool,
}

pub struct WorktreeScanner;
impl WorktreeScanner {
    pub fn scan(workspace: &SourceWorkspace) -> Result<Vec<WorktreeDescriptor>, DevMapError>;
}
```

- [ ] **Step 1: Write failing common-directory and linked-worktree tests.**

```rust
#[test]
fn linked_worktrees_share_common_dir_but_have_distinct_ids() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "codex/dock-agent");
    let main = SourceGitInspector::open(repo.path()).unwrap().workspace().unwrap();
    let other = SourceGitInspector::open(linked.path()).unwrap().workspace().unwrap();
    assert_eq!(main.git_common_dir, other.git_common_dir);
    let rows = WorktreeScanner::scan(&main).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|row| row.is_current).count(), 1);
    assert_ne!(rows[0].worktree_id, rows[1].worktree_id);
}
```

Also cover detached HEAD, locked/prunable markers, paths containing spaces, malformed duplicate fields, non-UTF-8 Git output, more than 256 entries, and a `.git` symlink/reparse-point substitution.

- [ ] **Step 2: Run the focused test and confirm the missing API failure.**

Run: `cargo test --test worktree_inventory -- --nocapture`

Expected: FAIL because `git_common_dir` and `WorktreeScanner` do not exist.

- [ ] **Step 3: Add Git common-directory discovery.**

Extend `SourceGitInspector::workspace()` with the read-only command below and resolve relative output against `root`:

```rust
let git_common_dir = self.required_git(["rev-parse", "--git-common-dir"])?;
```

Canonicalize only after checking every traversed component with the existing filesystem-security helpers. Do not create the returned directory and do not mutate Git configuration.

- [ ] **Step 4: Implement the bounded porcelain parser and stable ID.**

Run `git worktree list --porcelain -z` through the existing argument-array Git helper. Parse NUL-delimited records and reject duplicate required fields. Resolve each worktree's `.git` directory using a checked directory or a checked file such as `gitdir: C:/repo/.git/worktrees/agent-a`; compute:

```rust
let worktree_id = format!(
    "wt-{}",
    sha256_hex(format!("{}\0{}", repository_id, normalized_git_dir).as_bytes())
);
```

The display root may change without becoming the sole identity input. Sort the result by `worktree_id`; the reducer will apply UI ordering later.

- [ ] **Step 5: Run focused and Phase 1B Git regressions.**

Run: `cargo test --test worktree_inventory --test git_inspector --test journal_flow`

Expected: PASS with two distinct linked-worktree IDs and no Git metadata mutation.

- [ ] **Step 6: Commit Task 1.**

```bash
git add src/git.rs src/worktrees.rs src/lib.rs src/error.rs tests/worktree_inventory.rs tests/support/mod.rs
git commit -m "[FEAT](dock): discover linked worktrees"
```

### Task 2: Persist bounded disposable Presence records

**Files:**

- Create: `src/presence.rs`
- Modify: `src/events.rs`
- Modify: `src/hook.rs`
- Modify: `src/mcp.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/presence_store.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**

- Consumes: `SourceWorkspace.git_common_dir`, Phase 1B `JournalRecord`, `EventType`, canonical JSON, and filesystem-security primitives.
- Produces: `PresenceRecord`, `PresenceStore`, `PresenceSignal`, and status semantics consumed by `DockReducer`.

```rust
pub const MAX_PRESENCE_BYTES: usize = 64 * 1024;
pub const MAX_PRESENCE_RECORDS: usize = 2_048;
pub const DEFAULT_LEASE_SECONDS: i64 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceStatus { Starting, Working, Waiting, Idle, Completed, Stale, Unknown }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusSource { HostExplicit, CaptureEvent, Lease, GitOnly }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence { Observed, Leased, Inferred, Unknown }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresenceRecord {
    pub schema_version: u8,
    pub repository_id: String,
    pub worktree_id: String,
    pub session_id: String,
    pub actor_id: String,
    pub host: String,
    pub route_id: Option<String>,
    pub branch: Option<String>,
    pub head: String,
    pub status: PresenceStatus,
    pub status_source: StatusSource,
    pub confidence: Confidence,
    pub capture_grade: CaptureGrade,
    pub last_event_at: String,
    pub lease_expires_at: Option<String>,
    pub current_activity_id: Option<String>,
    pub current_decision_id: Option<String>,
    pub blocker_count: u32,
    pub gap_count: u32,
}

pub enum PresenceSignal<'a> {
    AcceptedRecords(&'a [JournalRecord]),
    ExplicitWaiting { session_id: &'a str, activity_id: Option<&'a str> },
}

impl PresenceStore {
    pub fn open(workspace: &SourceWorkspace) -> Result<Self, DevMapError>;
    pub fn observe(&self, signal: PresenceSignal<'_>, now: OffsetDateTime)
        -> Result<PresenceRecord, DevMapError>;
    pub fn load_all(&self) -> PresenceLoadReport;
}

impl PresenceRecord {
    pub fn effective_at(&self, now: OffsetDateTime) -> PresenceRecord;
}

pub fn project_status(
    previous: Option<PresenceStatus>,
    event_type: &EventType,
) -> PresenceStatus;

pub struct PresenceLoadReport {
    pub records: Vec<PresenceRecord>,
    pub warnings: Vec<PresenceWarning>,
    pub truncated: bool,
}

pub struct PresenceWarning {
    pub code: &'static str,
    pub subject_id: Option<String>,
}
```

- [ ] **Step 1: Write failing schema, transition, and privacy tests.**

```rust
#[test]
fn lease_expiry_is_stale_and_never_completed() {
    let mut record = support::presence_record(PresenceStatus::Working);
    record.lease_expires_at = Some("2026-09-02T12:00:00Z".into());
    let reduced = record.effective_at(OffsetDateTime::parse(
        "2026-09-02T12:00:01Z", &Rfc3339
    ).unwrap());
    assert_eq!(reduced.status, PresenceStatus::Stale);
    assert_eq!(reduced.status_source, StatusSource::Lease);
}
```

Add `tests/support::presence_record(status)` returning a valid record with fixed `repository_id`, `worktree_id`, `session_id`, actor, Codex host, lowercase forty-character HEAD, Grade D, RFC 3339 timestamps, zero counts, and no route/activity/decision. Cover SessionStart → `starting`, accepted activity → `working`, TurnCompleted → `idle`, explicit waiting signal → `waiting`, SessionEnd → `completed`, post-expiry → `stale`, and Git-only worktree → `unknown`. Serialize canary strings representing prompt, command, patch, tool input/output, and transcript fields and assert deserialization rejects every unknown field.

- [ ] **Step 2: Run the Presence test and confirm failure.**

Run: `cargo test --test presence_store -- --nocapture`

Expected: FAIL because `presence` does not exist.

- [ ] **Step 3: Implement canonical validation and atomic storage.**

Store records at:

```rust
workspace.git_common_dir
    .join("devmap/presence/v1")
    .join(format!("{}.json", checked_session_component))
```

Use canonical JSON, checked directory identities, a per-record lock, a same-directory temporary file, `sync_data`, atomic replacement, and parent-directory sync. Reject traversal, symlink/reparse substitution, files over 64 KiB, more than 2,048 directory entries, invalid RFC 3339 timestamps, wrong repository IDs, and invalid enum combinations such as `completed` with `status_source=lease`.

- [ ] **Step 4: Project accepted journal events into Presence.**

Map only validated events:

```rust
pub fn project_status(
    previous: Option<PresenceStatus>,
    event_type: &EventType,
) -> PresenceStatus {
    match event_type {
        EventType::SessionStarted => PresenceStatus::Starting,
        EventType::TurnCompleted => PresenceStatus::Idle,
        EventType::SessionStopped => PresenceStatus::Completed,
        EventType::ToolRequested
        | EventType::ToolCompleted
        | EventType::MutationObserved
        | EventType::DecisionRecorded
        | EventType::EvidenceRecorded
        | EventType::AgentStarted
        | EventType::AgentStopped
        | EventType::ContextCompacting
        | EventType::ContextCompacted => PresenceStatus::Working,
        _ => previous.unwrap_or(PresenceStatus::Working),
    }
}
```

Only `PresenceSignal::ExplicitWaiting` produces `waiting`. Completed records have no lease. Clamp `gap_count` and `blocker_count` at `u32::MAX`; never derive blocker text from payloads.

- [ ] **Step 5: Wire best-effort projection after authoritative capture.**

In `hook::handle_hook` and successful MCP record-tool handling, call `PresenceStore::observe` only after the journal append succeeds. A Presence failure writes a concise diagnostic to stderr but does not roll back, alter, or reclassify the accepted journal record. Add a test with an unwritable/replaced Presence directory proving the journal remains valid and the Hook response remains successful.

```rust
if let Err(error) = PresenceStore::open(&workspace)
    .and_then(|store| store.observe(PresenceSignal::AcceptedRecords(&records), now))
{
    eprintln!("devmap: presence update skipped: {error}");
}
```

- [ ] **Step 6: Run focused capture and Presence regressions.**

Run: `cargo test --test presence_store --test hook_flow --test mcp_stdio --test journal_flow`

Expected: PASS; canonical capture remains authoritative when Presence is unavailable.

- [ ] **Step 7: Commit Task 2.**

```bash
git add src/presence.rs src/events.rs src/hook.rs src/mcp.rs src/lib.rs src/error.rs tests/presence_store.rs tests/support/mod.rs
git commit -m "[FEAT](presence): project local agent status"
```

### Task 3: Build the revisioned Dock read model and `agents --json`

**Files:**

- Create: `src/dock.rs`
- Modify: `src/journal.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/dock_model.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**

- Consumes: `WorktreeDescriptor`, `PresenceLoadReport`, and read-only journal summaries.
- Produces: the exact `DockReadModel`, `DockService`, `RouteProvider`, and CLI JSON contract used by both transports and the UI.

```rust
pub const DOCK_SCHEMA_VERSION: &str = "devmap/dock/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockEntry {
    pub worktree_id: String,
    pub display_path: String,
    pub is_current: bool,
    pub branch: Option<String>,
    pub head: String,
    pub session_id: Option<String>,
    pub actor_id: Option<String>,
    pub host: Option<String>,
    pub route_id: Option<String>,
    pub status: PresenceStatus,
    pub status_source: StatusSource,
    pub confidence: Confidence,
    pub capture_grade: Option<CaptureGrade>,
    pub last_event_at: Option<String>,
    pub blocker_count: u32,
    pub gap_count: u32,
    pub capture_incomplete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockReadModel {
    pub schema_version: &'static str,
    pub repository_id: String,
    pub revision: u64,
    pub generated_at: String,
    pub current_worktree_id: String,
    pub current: Vec<DockEntry>,
    pub active: Vec<DockEntry>,
    pub stale_or_uninstrumented: Vec<DockEntry>,
    pub warnings: Vec<DockWarning>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockWarning {
    pub code: String,
    pub subject_id: Option<String>,
}

pub trait RouteProvider {
    fn route_for(&self, worktree_id: &str, session_id: Option<&str>) -> Option<String>;
}

pub struct NoRoutes;

pub struct DockReducer<R: RouteProvider> { routes: R }
impl<R: RouteProvider> DockReducer<R> {
    pub fn new(routes: R) -> Self;
    pub fn reduce(
        &self,
        workspace: &SourceWorkspace,
        worktrees: Vec<WorktreeDescriptor>,
        presence: PresenceLoadReport,
        journals: BTreeMap<String, JournalSummary>,
        now: OffsetDateTime,
    ) -> Result<DockReadModel, DevMapError>;
}

impl DockService {
    pub fn open(source: &Path) -> Result<Self, DevMapError>;
    pub fn refresh(&mut self, now: OffsetDateTime) -> Result<&DockReadModel, DevMapError>;
    pub fn snapshot(&self) -> &DockReadModel;
}

impl DockReadModel {
    pub fn content_hash(&self) -> Result<String, DevMapError>;
}
```

- [ ] **Step 1: Add a read-only journal summary contract and failing tests.**

```rust
pub struct JournalSummary {
    pub session_id: String,
    pub records: u64,
    pub last_sequence: Option<u64>,
    pub last_sha256: Option<String>,
    pub integrity: JournalIntegrity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalIntegrity { Verified, Missing, Corrupt }

pub fn summarize_existing_sessions(
    workspace: &SourceWorkspace,
    session_ids: &BTreeSet<String>,
) -> BTreeMap<String, JournalSummary>;
```

The function must not create missing session directories. It validates bounded canonical index/tail data and marks corrupt or missing referenced journals without modifying them. Test valid, missing, corrupt, oversized, and replaced journal files.

- [ ] **Step 2: Write failing reducer tests.**

```rust
#[test]
fn reducer_puts_current_first_and_unknown_worktrees_in_warning_group() {
    let fixture = support::dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes).reduce(
        &fixture.workspace,
        fixture.worktrees,
        fixture.presence,
        fixture.journals,
        fixture.now,
    ).unwrap();
    assert!(model.current[0].is_current);
    assert_eq!(model.active.len(), 1);
    assert_eq!(model.stale_or_uninstrumented[0].status, PresenceStatus::Unknown);
}
```

Define `tests/support::DockReducerFixture` with the five fields passed above and a `dock_reducer_fixture()` constructor using two valid worktree descriptors, one active Presence record, one verified journal summary, and a fixed timestamp. Cover multiple Agents in one worktree, stale records, unknown worktrees, mismatched repository/worktree IDs, corrupt Presence, corrupt journal, deterministic ordering, `CAPTURE INCOMPLETE`, warning counts, truncation, and identical canonical graph fixtures before and after overlay reduction.

- [ ] **Step 3: Run the reducer tests and confirm failure.**

Run: `cargo test --test dock_model -- --nocapture`

Expected: FAIL because `DockReadModel` and the journal summary API are absent.

- [ ] **Step 4: Implement deterministic reduction and revision changes.**

Build a content hash from the model excluding `revision` and `generated_at`. `DockService::refresh` increments `revision` only when that hash changes; process restart begins at revision `1`. Sort current first, then severity (`waiting`, `stale`, `working`, `starting`, `idle`, `completed`, `unknown`), newest activity descending, then stable worktree/session ID. `NoRoutes` returns `None` without fabricating a route.

```rust
let content_hash = next.content_hash()?;
if self.content_hash.as_deref() != Some(&content_hash) {
    self.revision = self.revision.checked_add(1)
        .ok_or(DevMapError::DockRevisionOverflow)?;
    self.content_hash = Some(content_hash);
}
next.revision = self.revision.max(1);
```

- [ ] **Step 5: Add `devmap agents --source PATH --json`.**

```rust
#[derive(Debug, Args)]
pub struct AgentsArgs {
    #[arg(long, default_value = ".")]
    pub source: PathBuf,
    #[arg(long)]
    pub json: bool,
}
```

JSON prints the canonical `DockReadModel`. Text mode prints bounded columns for Current/Active/Stale-or-Uninstrumented and visible warning totals. Neither mode starts a listener or writes source/Context Git state.

- [ ] **Step 6: Run focused model, CLI, and immutability tests.**

Run: `cargo test --test dock_model --test cli_help --test phase_1b_acceptance`

Expected: PASS; `agents` appears in help and source Git snapshots are unchanged.

- [ ] **Step 7: Commit Task 3.**

```bash
git add src/dock.rs src/journal.rs src/cli.rs src/lib.rs src/error.rs tests/dock_model.rs tests/cli_help.rs tests/support/mod.rs
git commit -m "[FEAT](dock): build revisioned agent read model"
```

### Task 4: Add the shared dependency-free Dock UI contract

**Files:**

- Create: `assets/dock.html`
- Create: `src/dock_asset.rs`
- Modify: `src/lib.rs`
- Create: `tests/dock_ui_contract.rs`

**Interfaces:**

- Consumes: serialized `DockReadModel` from Task 3.
- Produces: `DOCK_RESOURCE_URI`, `DOCK_MIME_TYPE`, `dock_html()`, MCP Apps bridge handling, Browser fetch/SSE handling, and portable selection context.

```rust
pub const DOCK_RESOURCE_URI: &str = "ui://devmap/dock/v1.html";
pub const DOCK_MIME_TYPE: &str = "text/html;profile=mcp-app";
pub fn dock_html() -> &'static str { include_str!("../assets/dock.html") }
```

- [ ] **Step 1: Write failing static UI-contract tests.**

```rust
#[test]
fn dock_asset_is_self_contained_and_uses_portable_bridge() {
    let html = dock_html();
    assert!(html.contains("ui/initialize"));
    assert!(html.contains("ui/notifications/tool-result"));
    assert!(html.contains("ui/update-model-context"));
    assert!(html.contains("devmap_dock_snapshot"));
    assert!(!html.contains("https://"));
    assert!(!html.contains("http://"));
    assert!(!html.contains("localStorage"));
}
```

Also assert the document contains one `main` landmark, keyboard-operable buttons, `aria-live`, explicit Current/Active/Stale groups, visible status/confidence labels, `CAPTURE INCOMPLETE`, an offline/age region, CSS for narrow and wide containers, reduced-motion rules, and no raw-data field names.

- [ ] **Step 2: Run the UI-contract test and confirm failure.**

Run: `cargo test --test dock_ui_contract -- --nocapture`

Expected: FAIL because the asset module does not exist.

- [ ] **Step 3: Implement one self-contained transport-adaptive document.**

The JavaScript accepts initial data from `ui/notifications/tool-result`. While visible, MCP mode calls `devmap_dock_snapshot` through the portable `tools/call` bridge no faster than every two seconds. Browser mode reads the bootstrapped `/api/v1/dock/snapshot` URL and then attaches `EventSource` to `/api/v1/dock/events`. Stop polling/reconnect timers when `document.visibilityState !== "visible"`.

```javascript
let refreshTimer;
let nextRequestId = 1;
const pendingRequests = new Map();
const transport = window.parent === window ? "browser" : "mcp";

function callTool(name, args) {
  const id = nextRequestId++;
  window.parent.postMessage({
    jsonrpc: "2.0", id, method: "tools/call",
    params: { name, arguments: args }
  }, "*");
  return id;
}

function scheduleRefresh() {
  clearTimeout(refreshTimer);
  if (document.visibilityState === "visible" && transport === "mcp") {
    refreshTimer = setTimeout(() => callTool("devmap_dock_snapshot", {}), 2000);
  }
}

window.addEventListener("message", (event) => {
  if (event.source !== window.parent) return;
  const message = event.data;
  if (message?.jsonrpc !== "2.0") return;
  if (message.method === "ui/notifications/tool-result") {
    acceptSnapshot(message.params?.structuredContent);
  }
});
```

On row selection, update only ephemeral UI state and send:

```json
{
  "method": "ui/update-model-context",
  "params": {
    "content": [{
      "type": "text",
      "text": "DevMap selection: worktree_id=wt-0123456789abcdef route_id=route-or-none"
    }]
  }
}
```

Do not send absolute paths, session IDs, prompts, or evidence content through model context. Browser mode stores selection only in memory.

- [ ] **Step 4: Implement resilient rendering.**

Treat every tool result and HTTP payload as untrusted. Reject the wrong `schema_version`, non-integer revision, oversized arrays, and missing IDs in JavaScript before rendering. Preserve the last valid snapshot on transport failure, show `OFFLINE · last update {age}`, and discard any response with a revision lower than the currently rendered revision.

```javascript
function acceptSnapshot(value) {
  if (!value || value.schema_version !== "devmap/dock/1") return false;
  if (!Number.isSafeInteger(value.revision) || value.revision < renderedRevision) return false;
  for (const key of ["current", "active", "stale_or_uninstrumented"]) {
    if (!Array.isArray(value[key]) || value[key].length > 2048) return false;
    if (value[key].some((row) => typeof row.worktree_id !== "string")) return false;
  }
  renderedRevision = value.revision;
  renderSnapshot(value);
  return true;
}
```

Define `renderSnapshot(value)` in the same document to replace only text nodes and DOM attributes created by the application; never assign untrusted strings to `innerHTML`.

- [ ] **Step 5: Run UI contract and model serialization tests.**

Run: `cargo test --test dock_ui_contract --test dock_model`

Expected: PASS; the asset contains no off-machine URL and accepts the Task 3 JSON shape.

- [ ] **Step 6: Commit Task 4.**

```bash
git add assets/dock.html src/dock_asset.rs src/lib.rs tests/dock_ui_contract.rs
git commit -m "[FEAT](dock): add shared live dock interface"
```

### Task 5: Expose Dock data and UI through the existing STDIO MCP server

**Files:**

- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/mcp.rs`
- Modify: `src/dock.rs`
- Create: `tests/dock_mcp.rs`
- Modify: `tests/mcp_stdio.rs`

**Interfaces:**

- Consumes: `DockService`, `dock_html`, and the existing Phase 1B MCP parser/tool dispatcher.
- Produces: read-only `devmap_dock_snapshot`, render-only `devmap_open_dock`, `resources/list`, and `resources/read` while preserving all four existing capture tools.

```rust
pub const DOCK_DATA_TOOL: &str = "devmap_dock_snapshot";
pub const DOCK_RENDER_TOOL: &str = "devmap_open_dock";

pub struct McpRuntime {
    workspace: SourceWorkspace,
    dock: DockService,
    audit: TransportAudit,
}

impl McpRuntime {
    pub fn open(source: &Path) -> Result<Self, DevMapError>;
    pub fn handle(&mut self, message: &Value) -> Option<Value>;
    pub fn audit(&self) -> &TransportAudit;
}

#[derive(Debug, Args)]
pub struct McpArgs {
    #[arg(long, default_value = ".")]
    pub source: PathBuf,
}
```

- [ ] **Step 1: Write failing MCP resource and tool tests.**

Send real newline-delimited JSON-RPC through `serve_mcp` and assert:

```rust
assert_eq!(resource["uri"], "ui://devmap/dock/v1.html");
assert_eq!(resource["mimeType"], "text/html;profile=mcp-app");
assert_eq!(render_tool["_meta"]["ui"]["resourceUri"], resource["uri"]);
assert_eq!(snapshot_tool["annotations"]["readOnlyHint"], true);
```

Verify `devmap_dock_snapshot` has no UI resource, `devmap_open_dock` alone owns `_meta.ui.resourceUri`, both reject arguments, `resources/read` rejects any other URI, existing capture tools retain their descriptors, and legacy/modern protocol metadata remains valid.

- [ ] **Step 2: Run focused MCP tests and confirm failure.**

Run: `cargo test --test dock_mcp --test mcp_stdio -- --nocapture`

Expected: FAIL because Dock resources and tools are not advertised.

- [ ] **Step 3: Add read-only resource methods.**

Extend dispatch with `resources/list` and `resources/read`. Return one resource with embedded HTML and metadata:

```rust
json!({
    "uri": DOCK_RESOURCE_URI,
    "mimeType": DOCK_MIME_TYPE,
    "text": dock_html(),
    "_meta": {
        "ui": {
            "prefersBorder": false,
            "csp": {
                "connectDomains": [],
                "resourceDomains": [],
                "frameDomains": []
            }
        }
    }
})
```

Keep the HTML under the existing 1 MiB MCP line limit and return a typed resource-limit error if a future asset exceeds it.

- [ ] **Step 4: Add decoupled data and render tools.**

`devmap_dock_snapshot` refreshes `DockService` and returns the complete model in `structuredContent`. `devmap_open_dock` returns the same initial model plus `_meta.ui.resourceUri`; its text content is a short accessible summary, not raw JSON. Both tools are read-only, closed-world, and non-destructive. Neither calls `viewer::serve`, `TcpListener::bind`, or any Browser-launch function.

```rust
fn dock_tool_descriptor(name: &str, renders_ui: bool) -> Value {
    let mut descriptor = json!({
        "name": name,
        "description": "Read the current local DevMap worktree state.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        "annotations": {"readOnlyHint": true, "openWorldHint": false, "destructiveHint": false}
    });
    if renders_ui {
        descriptor["_meta"] = json!({"ui": {"resourceUri": DOCK_RESOURCE_URI}});
    }
    descriptor
}
```

- [ ] **Step 5: Verify STDIO-only lifecycle and bounded visible refresh.**

Add a `TransportAudit` test seam owned by `McpRuntime`:

```rust
#[derive(Default)]
pub struct TransportAudit {
    pub stdio_messages: u64,
    pub tcp_listeners_opened: u64,
}
```

The production MCP path can increment only `stdio_messages`; no viewer type is a dependency of `mcp.rs`. Feed three snapshot tool calls representing visible-only refresh and assert increasing or equal revisions, response bounds, and `tcp_listeners_opened == 0`.

- [ ] **Step 6: Run all MCP and capture regressions.**

Run: `cargo test --test dock_mcp --test mcp_stdio --test final_review_mcp --test phase_1b_acceptance`

Expected: PASS with six tools total and no change to the four capture-tool behaviors.

- [ ] **Step 7: Commit Task 5.**

```bash
git add src/cli.rs src/lib.rs src/mcp.rs src/dock.rs tests/dock_mcp.rs tests/mcp_stdio.rs
git commit -m "[FEAT](mcp): serve live dock over stdio"
```

### Task 6: Package the Codex plugin for host-managed startup

**Files:**

- Create: `plugins/devmap/.codex-plugin/plugin.json`
- Create: `plugins/devmap/.mcp.json`
- Create: `plugins/devmap/skills/live-worktree-dock/SKILL.md`
- Create: `tests/dock_plugin.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`

**Interfaces:**

- Consumes: the `devmap mcp` command and Dock MCP tools from Task 5.
- Produces: an installable repository plugin whose configured command is launched by Codex without a manual HTTP server.

- [ ] **Step 1: Write failing manifest and real-launch tests.**

Parse both JSON files with `serde_json`. Assert the manifest points to `./.mcp.json` and the MCP entry is exactly one enabled STDIO server with command `devmap`, arguments `mcp`, ten-second startup/tool timeouts, and read-only auto-approval only for the two Dock tools. Launch the configured command in a disposable Git repository, send `initialize` and `tools/list`, then assert the Dock tools appear without running `devmap view --live`.

- [ ] **Step 2: Run the plugin test and confirm failure.**

Run: `cargo test --test dock_plugin -- --nocapture`

Expected: FAIL because `plugins/devmap` does not exist.

- [ ] **Step 3: Add the minimal plugin manifest.**

```json
{
  "name": "devmap",
  "version": "0.1.0",
  "description": "Evidence-backed development maps and live local worktree presence.",
  "mcpServers": "./.mcp.json",
  "skills": "./skills/"
}
```

- [ ] **Step 4: Add the host-managed STDIO definition.**

```json
{
  "mcpServers": {
    "devmap": {
      "command": "devmap",
      "args": ["mcp"],
      "enabled": true,
      "startup_timeout_sec": 10,
      "tool_timeout_sec": 10,
      "default_tools_approval_mode": "writes",
      "tools": {
        "devmap_dock_snapshot": { "approval_mode": "auto" },
        "devmap_open_dock": { "approval_mode": "auto" }
      }
    }
  }
}
```

Do not set `cwd`, a localhost URL, or an absolute repository path. `devmap mcp --source .` uses the Codex task's process working directory; users who register the server outside a task may provide an explicit `--source PATH` override in their own configuration.

- [ ] **Step 5: Add the narrowly triggered Dock skill.**

The skill frontmatter name is `live-worktree-dock`. Its description triggers only when the user asks to show/open/refresh the DevMap Dock or inspect current/other local worktree Agents. Its body says to call `devmap_open_dock` once, use `devmap_dock_snapshot` for explicit text-only inspection, never claim cross-machine coverage, and never tell the user to start an HTTP server in Codex.

- [ ] **Step 6: Document installation versus runtime behavior.**

In both READMEs state: installation/enabling is one-time; Codex launches `devmap mcp`; opening the MCP App requires no `devmap view --live`; Browser fallback is optional and temporary; project trust or managed MCP policy may disable the plugin and must be surfaced honestly.

- [ ] **Step 7: Run plugin, help, and adapter regression tests.**

Run: `cargo test --test dock_plugin --test cli_help --test adapter_install --test adapter_conformance`

Expected: PASS; the configured STDIO command launches in the fixture repository and existing adapters remain valid.

- [ ] **Step 8: Commit Task 6.**

```bash
git add plugins/devmap README.md README.zh-CN.md tests/dock_plugin.rs
git commit -m "[FEAT](plugin): package zero-start Codex dock"
```

### Task 7: Add the on-demand loopback Browser fallback

**Files:**

- Modify: `Cargo.toml`
- Create: `src/viewer.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/dock_viewer.rs`

**Interfaces:**

- Consumes: `DockService` and `dock_html`.
- Produces: explicit `devmap view --live --source PATH`, authenticated snapshot/SSE/health endpoints, and a stoppable test handle.

```rust
#[derive(Debug, Args)]
pub struct ViewArgs {
    #[arg(long, default_value = ".")]
    pub source: PathBuf,
    #[arg(long)]
    pub live: bool,
}

pub struct ViewerHandle {
    pub address: SocketAddr,
    pub token: String,
}

pub struct ViewerRuntime {
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<(), DevMapError>>>,
}

impl ViewerRuntime {
    pub fn shutdown(self) -> Result<(), DevMapError>;
}

pub fn start_live_viewer(
    source: &Path,
    bind: SocketAddr,
) -> Result<(ViewerHandle, ViewerRuntime), DevMapError>;
```

- [ ] **Step 1: Write failing HTTP-boundary tests using raw `TcpStream`.**

Start with `127.0.0.1:0` and assert:

- no token → `401`;
- wrong token → `401`;
- valid `GET /api/v1/health?token=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef` → `200` for the fixture token;
- valid snapshot → `application/json`, `Cache-Control: no-store`, and Task 3 schema;
- valid events → `text/event-stream`, an initial `snapshot` event, monotonic revision IDs, and reconnect from `after`;
- POST/PUT/DELETE → `405`;
- unknown and traversal-like paths → `404` without filesystem access;
- bind address other than loopback → typed refusal;
- shutdown closes the listening socket.

- [ ] **Step 2: Run the Viewer test and confirm failure.**

Run: `cargo test --test dock_viewer -- --nocapture`

Expected: FAIL because `viewer` and `view --live` do not exist.

- [ ] **Step 3: Add only the two required runtime dependencies.**

```toml
getrandom = "0.3"
tiny_http = "0.12"
```

Generate 32 random bytes with `getrandom::fill`, hex-encode them without another crate, and keep the token only in memory and the printed URL. Never persist it in Presence or logs.

- [ ] **Step 4: Implement the read-only server and SSE stream.**

Serve only:

```text
GET /?token=<token>
GET /api/v1/health?token=<token>
GET /api/v1/dock/snapshot?token=<token>
GET /api/v1/dock/events?token=<token>&after=<revision>
```

Use `recv_timeout` for shutdown and a bounded per-client channel for SSE. Slow clients receive a new complete snapshot after reconnect rather than an unbounded backlog. Refresh the reducer no faster than every 500 ms and send only revisions greater than `after`.

- [ ] **Step 5: Dispatch `view --live` without changing the MCP path.**

`devmap view --live` prints the authenticated URL once to stdout and serves until its owning CLI process exits. `devmap view` without `--live` returns a typed message pointing to the future canonical topology Viewer; do not silently start a different feature. Confirm the `Command::Mcp` arm imports no Viewer type and starts no listener.

```rust
match args.live {
    true => viewer::run_live(&args.source),
    false => Err(DevMapError::UnsupportedCommand("canonical topology viewer")),
}
```

- [ ] **Step 6: Run Viewer, MCP-isolation, and security tests.**

Run: `cargo test --test dock_viewer --test dock_mcp --test final_review_installer --test final_review_journal`

Expected: PASS; Browser endpoints are token-protected and MCP remains STDIO-only.

- [ ] **Step 7: Commit Task 7.**

```bash
git add Cargo.toml Cargo.lock src/viewer.rs src/cli.rs src/lib.rs src/error.rs tests/dock_viewer.rs
git commit -m "[FEAT](viewer): add on-demand dock fallback"
```

### Task 8: Prove multi-worktree behavior, limits, and visual acceptance

**Files:**

- Create: `tests/live_dock_acceptance.rs`
- Modify: `tests/support/mod.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Create: `.superpowers/sdd/2026-09-02-devmap-live-worktree-dock-mvp/verification-report.md`

**Interfaces:**

- Consumes: all Tasks 1–7.
- Produces: the executable MVP acceptance gate and operator instructions.

- [ ] **Step 1: Build a failing end-to-end two-worktree scenario.**

Call a not-yet-created `support::live_dock_fixture()` to create `main` plus `codex/agent-a` and `codex/agent-b`. Feed real Codex, Claude, and Generic MCP events into separate sessions. Assert one shared read model shows the current worktree first, active Agents in other worktrees, one uninstrumented worktree as `unknown`, one expired session as `stale`, one explicit SessionEnd as `completed`, and no fabricated route when `NoRoutes` is active.

- [ ] **Step 2: Add privacy, corruption, and source-immutability assertions.**

Place unique prompt/command/patch/tool/transcript canaries in accepted fixture inputs; recursively inspect Presence, `agents --json`, MCP tool output, MCP resource HTML, HTTP snapshot, and SSE output and assert no canary appears. Corrupt one Presence record and one journal index and assert `PRESENCE INCOMPLETE`/`capture_incomplete` appears without changing source HEAD, branch, index, refs, config, stash, remotes, or worktree registration.

```rust
for canary in [
    "PROMPT_CANARY_91D2", "COMMAND_CANARY_91D2", "PATCH_CANARY_91D2",
    "TOOL_INPUT_CANARY_91D2", "TOOL_OUTPUT_CANARY_91D2", "TRANSCRIPT_CANARY_91D2",
] {
    assert!(!all_observable_outputs.contains(canary), "leaked {canary}");
}
assert_eq!(support::source_snapshot(fixture.repo.path()).git, before.git);
```

- [ ] **Step 3: Run the end-to-end test and confirm any remaining gaps fail.**

Run: `cargo test --test live_dock_acceptance -- --nocapture`

Expected: FAIL to compile because `support::live_dock_fixture` does not exist.

- [ ] **Step 4: Implement the exact reusable acceptance fixture.**

```rust
pub struct LiveDockFixture {
    pub repo: TempDir,
    pub agent_a: TempDir,
    pub agent_b: TempDir,
}

pub fn live_dock_fixture() -> LiveDockFixture {
    let repo = committed_repo();
    let agent_a = linked_worktree(repo.path(), "codex/agent-a");
    let agent_b = linked_worktree(repo.path(), "codex/agent-b");
    LiveDockFixture { repo, agent_a, agent_b }
}
```

Re-run `cargo test --test live_dock_acceptance -- --nocapture`; expected: PASS. Any failure must be fixed inside the Task 1–7 modules without changing their public signatures or adding route reconstruction, cross-machine Presence, or canonical graph writes.

- [ ] **Step 5: Run the performance gate in release mode.**

Generate 100 linked-worktree descriptors and 1,000 bounded Presence records without running 100 real Git worktree commands. Time scanner-fixture parsing plus store load plus reduction:

```rust
assert!(elapsed < Duration::from_secs(1));
```

Then append one accepted event and assert a polling/SSE-visible revision is available within two seconds. Print measured durations for CI diagnosis.

- [ ] **Step 6: Perform manual Codex MCP App and Browser visual acceptance.**

Install the repository plugin through a local marketplace, open a fresh Codex task rooted at the implementation worktree, and ask to show the DevMap Live Worktree Dock. Record proof in `.superpowers/sdd/2026-09-02-devmap-live-worktree-dock-mvp/verification-report.md` that Codex launched `devmap mcp`, rendered the MCP App, and did not require or launch `devmap view --live` or a TCP listener. Then run `cargo run --release -- view --live --source .`, open the printed URL in the Codex Browser, and record the fallback checklist: 320 px and 520 px pane widths, keyboard traversal, focus visibility, screen-reader labels, current-worktree emphasis, collapsed stale group, offline age, `CAPTURE INCOMPLETE`, reduced motion, and row selection. This is the only planned manual step; it supplements rather than replaces automated tests.

- [ ] **Step 7: Update bilingual usage and limitation sections.**

Document exact commands for `devmap agents --json` and Browser fallback, the zero-manual-start Codex plugin path, Presence location, state meanings, local-machine boundary, failure banners, and the deferred route/topology integration.

- [ ] **Step 8: Run the full verification gate.**

Run:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
cargo run -- --help
cargo run -- agents --help
cargo run -- view --help
cargo run -- mcp --help
git diff --check
git status --short
```

Expected: all commands exit `0`; all tests pass; `git status --short` lists only intended implementation/report changes plus the pre-existing untracked `work/`.

- [ ] **Step 9: Commit Task 8.**

```bash
git add tests/live_dock_acceptance.rs tests/support/mod.rs README.md README.zh-CN.md .superpowers/sdd/2026-09-02-devmap-live-worktree-dock-mvp/verification-report.md
git commit -m "[TEST](dock): verify live worktree experience"
```

## Final review gate

- [ ] Map design sections 1–15 to a passing test, an implementation file, or the explicit post-MVP scope boundary.
- [ ] Confirm `completed` appears only after `SessionStopped`; force lease expiry and verify `stale`.
- [ ] Confirm corrupt/oversized/replaced Presence records cannot crash the Dock or escape response limits.
- [ ] Confirm no prompt, command, patch, tool input/output, file content, or transcript canary crosses the Presence/read-model/UI boundary.
- [ ] Confirm the MCP server launches from the plugin configuration and does not open a TCP listener.
- [ ] Confirm Browser fallback starts only on demand, binds loopback, requires its token, exposes only GET, and exits with its owner.
- [ ] Confirm current/other worktrees and Codex/Claude/Generic MCP sessions render in one unified ordering.
- [ ] Confirm Presence changes do not alter canonical graph fixtures or any source/Context Git state.
- [ ] Confirm the release performance gates meet one-second initial reduction and two-second visible update targets.
- [ ] Confirm `work/` was neither modified nor staged.
- [ ] Request code review before merging the implementation branch.
