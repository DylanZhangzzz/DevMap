# DevMap Worktree Conversation Panorama Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep each worktree as the primary Git lane while making every visible conversation and its observed agent immediately readable on a horizontally pannable topology canvas.

**Architecture:** Preserve the existing `devmap/dock/2` Rust model and implement the feature inside the self-contained Dock asset. Split the viewport into a sticky worktree identity layer and a wide scroll surface; render ordered conversation nodes on each worktree rail, then add bounded history disclosure and input-safe horizontal navigation.

**Tech Stack:** Rust integration tests, self-contained HTML/CSS/vanilla JavaScript, Codex in-app Browser Design QA.

**Spec:** `docs/superpowers/specs/2026-09-04-devmap-worktree-conversation-panorama-design.md`

## Global Constraints

- Worktree remains the first-level lane identity.
- Conversation title, truthful observed `actor_id`, and activity state are visible in MAP, READ, and FULL.
- The UI must not imply a complete multi-agent roster when only one observed actor is available.
- The `devmap/dock/2` Rust serialization schema does not change.
- Only the topology viewport may scroll horizontally; the document must not overflow horizontally.
- All untrusted model values are rendered through DOM text nodes, never `innerHTML`.
- Drag panning must ignore buttons, links, text-selection areas, and scrollbars.
- Native trackpad scrolling, Shift + wheel, pointer drag, bottom scrollbar, and keyboard scrolling remain available.
- Existing refresh, selection, validation, density, and portable route behavior remain intact.
- Existing truthful `route_id` values continue to drive portable conversation selection context; missing routes are never invented.

---

### Task 1: Lock the worktree → conversation → agent hierarchy in contract tests

**Files:**
- Modify: `tests/dock_ui_contract.rs`
- Test: `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: the embedded HTML returned by the existing Dock asset test helper.
- Produces: contract tests for `conversation-track`, `conversation-node`, `agent-identity`, `conversation-state`, `historical-conversations`, and density rules.

- [ ] **Step 1: Write failing hierarchy and density tests**

Add focused assertions equivalent to:

```rust
#[test]
fn dock_asset_keeps_worktrees_primary_and_conversations_visible_in_every_density() {
    let html = dock_html();
    assert!(html.contains("worktree-identity"));
    assert!(html.contains("conversation-track"));
    assert!(html.contains("conversation-node"));
    assert!(html.contains("agent-identity"));
    assert!(html.contains("conversation-state"));
    assert!(!html.contains("html[data-density=\"map\"] .conversation-track { display: none"));
}

#[test]
fn dock_asset_orders_and_bounds_historical_conversations() {
    let html = dock_html();
    assert!(html.contains("function compareConversations"));
    assert!(html.contains("const MAX_RECENT_INACTIVE = 3"));
    assert!(html.contains("historical-conversations"));
    assert!(html.contains("historical conversations"));
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```powershell
cargo test --test dock_ui_contract dock_asset_keeps_worktrees_primary_and_conversations_visible_in_every_density -- --nocapture
cargo test --test dock_ui_contract dock_asset_orders_and_bounds_historical_conversations -- --nocapture
```

Expected: both tests fail because the new conversation hierarchy and ordering helpers do not exist.

- [ ] **Step 3: Implement deterministic conversation ordering and disclosure**

In `assets/dock.html`, add:

```javascript
const MAX_RECENT_INACTIVE = 3;
const activeConversationStates = new Set(["starting", "working", "waiting"]);

function effectiveConversationState(chat) {
  return chat.host_status || chat.status;
}

function compareConversations(left, right) {
  const bucket = (chat) => activeConversationStates.has(effectiveConversationState(chat)) ? 0
    : effectiveConversationState(chat) === "idle" ? 1 : 2;
  return bucket(left) - bucket(right)
    || Date.parse(right.last_event_at) - Date.parse(left.last_event_at)
    || left.session_id.localeCompare(right.session_id);
}
```

Render every active conversation and the first three ordered inactive conversations. Put older inactive items behind a button with class `historical-conversations`, `aria-expanded`, and copy `+N historical conversations`.

Each visible item must use this semantic structure:

```html
<button class="conversation-node">
  <span class="conversation-title"></span>
  <span class="agent-identity"></span>
  <span class="conversation-state"></span>
</button>
```

Create the elements with `document.createElement` and assign every model value with `textContent`.

- [ ] **Step 4: Run the focused tests and verify GREEN**

Run:

```powershell
cargo test --test dock_ui_contract dock_asset_keeps_worktrees_primary_and_conversations_visible_in_every_density -- --nocapture
cargo test --test dock_ui_contract dock_asset_orders_and_bounds_historical_conversations -- --nocapture
```

Expected: both tests pass.

- [ ] **Step 5: Commit the hierarchy slice**

```powershell
git add -- assets/dock.html tests/dock_ui_contract.rs
git commit -m "[ENHANCE](dock): Promote conversation and agent visibility"
```

---

### Task 2: Introduce the panoramic worktree viewport

**Files:**
- Modify: `assets/dock.html`
- Modify: `tests/dock_ui_contract.rs`
- Test: `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: worktree bands and ordered conversation nodes from Task 1.
- Produces: `topology-viewport`, `topology-surface`, `worktree-identity`, `topologyWidth`, and document-safe horizontal overflow.

- [ ] **Step 1: Write failing panoramic-layout tests**

Add:

```rust
#[test]
fn dock_asset_uses_a_sticky_worktree_index_and_scoped_horizontal_viewport() {
    let html = dock_html();
    assert!(html.contains("topology-viewport"));
    assert!(html.contains("topology-surface"));
    assert!(html.contains("position: sticky"));
    assert!(html.contains("overflow-x: auto"));
    assert!(html.contains("scrollbar-gutter: stable"));
    assert!(html.contains("function topologyWidth"));
    assert!(html.contains("--topology-width"));
}
```

Keep the existing assertion that the page root does not horizontally overflow.

- [ ] **Step 2: Run the focused test and verify RED**

```powershell
cargo test --test dock_ui_contract dock_asset_uses_a_sticky_worktree_index_and_scoped_horizontal_viewport -- --nocapture
```

Expected: failure because the viewport, surface, and width helper are absent.

- [ ] **Step 3: Implement the wide surface and pinned identity column**

Wrap the rendered rail sections in:

```html
<div class="topology-viewport" id="topology-viewport" tabindex="0" aria-label="Scrollable repository topology">
  <div class="topology-surface" id="topology-surface"></div>
</div>
```

Use these layout contracts:

```css
.topology-viewport {
  position: relative;
  overflow-x: auto;
  overflow-y: hidden;
  scrollbar-gutter: stable;
  overscroll-behavior-inline: contain;
  cursor: grab;
}
.topology-viewport.is-panning { cursor: grabbing; user-select: none; }
.topology-surface { min-width: var(--topology-width); }
.worktree-identity {
  position: sticky;
  left: 0;
  z-index: 6;
  background: var(--bg-canvas);
}
```

Add a bounded pure helper:

```javascript
function topologyWidth(groups) {
  const forks = Math.max(1, groups.length);
  const conversations = Math.max(1, ...groups.flatMap((group) => group.lanes.map((lane) => lane.chats.length)));
  return Math.min(6400, Math.max(1080, 760 + forks * 150 + conversations * 240));
}
```

Apply the result only as a numeric pixel custom property generated by trusted code:

```javascript
byId("topology-surface").style.setProperty("--topology-width", `${topologyWidth(value.branch_groups)}px`);
```

- [ ] **Step 4: Run the focused test and verify GREEN**

```powershell
cargo test --test dock_ui_contract dock_asset_uses_a_sticky_worktree_index_and_scoped_horizontal_viewport -- --nocapture
```

Expected: pass.

- [ ] **Step 5: Commit the panoramic-layout slice**

```powershell
git add -- assets/dock.html tests/dock_ui_contract.rs
git commit -m "[ENHANCE](dock): Add panoramic worktree viewport"
```

---

### Task 3: Add safe mouse, wheel, scrollbar, and keyboard navigation

**Files:**
- Modify: `assets/dock.html`
- Modify: `tests/dock_ui_contract.rs`
- Test: `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: `#topology-viewport` from Task 2.
- Produces: `installPanControls(viewport)`, safe interactive-descendant exclusion, pointer capture, and horizontal keyboard navigation.

- [ ] **Step 1: Write failing interaction contract tests**

Add:

```rust
#[test]
fn dock_asset_supports_safe_horizontal_pan_inputs() {
    let html = dock_html();
    assert!(html.contains("function installPanControls"));
    assert!(html.contains("pointerdown"));
    assert!(html.contains("setPointerCapture"));
    assert!(html.contains("event.shiftKey"));
    assert!(html.contains("event.target.closest(\"button, a, input, textarea, select, [data-no-pan]\")"));
    assert!(html.contains("ArrowLeft"));
    assert!(html.contains("ArrowRight"));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

```powershell
cargo test --test dock_ui_contract dock_asset_supports_safe_horizontal_pan_inputs -- --nocapture
```

Expected: failure because no pan controller exists.

- [ ] **Step 3: Implement input-safe pan controls**

Add `installPanControls(viewport)` with one private pointer state object. On primary-button `pointerdown`, return immediately when the target is within `button, a, input, textarea, select, [data-no-pan]`; otherwise record `clientX` and `scrollLeft`, capture the pointer, and add `is-panning`. On `pointermove`, set `scrollLeft` to the starting value minus the pointer delta. On `pointerup`, `pointercancel`, and `lostpointercapture`, clear the state and class.

On `wheel`, translate only Shift + vertical wheel input into horizontal movement and call `preventDefault()` only when horizontal movement is possible. Do not interfere with native horizontal trackpad input.

On `keydown`, support `ArrowLeft`, `ArrowRight`, `Home`, and `End` while the viewport has focus. Use instant scrolling when `prefers-reduced-motion: reduce` matches.

- [ ] **Step 4: Run the focused test and verify GREEN**

```powershell
cargo test --test dock_ui_contract dock_asset_supports_safe_horizontal_pan_inputs -- --nocapture
```

Expected: pass.

- [ ] **Step 5: Commit the navigation slice**

```powershell
git add -- assets/dock.html tests/dock_ui_contract.rs
git commit -m "[ENHANCE](dock): Add safe horizontal pan controls"
```

---

### Task 4: Complete browser Design QA and update evidence

**Files:**
- Modify: `design-qa.md`
- Create: `.superpowers/brainstorm/product-design/conversation-panorama-final.jpg`
- Create: `.superpowers/brainstorm/product-design/comparison-conversation-panorama-final.png`
- Test: `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: the completed Dock asset from Tasks 1–3 and real local repository/task inventory.
- Produces: visual evidence, interaction measurements, and a final QA record.

- [ ] **Step 1: Run the complete Dock UI contract suite**

```powershell
cargo test --test dock_ui_contract -- --nocapture
```

Expected: every Dock UI contract test passes.

- [ ] **Step 2: Start a fresh local viewer and open it in the Codex in-app Browser**

```powershell
cargo run -- view --source "C:\Users\user\Documents\ChatGPT\DevMap-phase-1a-worktree" --live
```

Use the generated loopback URL only inside the Browser tool. Do not expose its token in user-facing text.

- [ ] **Step 3: Verify visual hierarchy and interactions in Browser**

With a snapshot containing linked conversations, verify:

```javascript
({
  brand: document.querySelector('.eyebrow')?.textContent.trim(),
  title: document.querySelector('h1')?.textContent.trim(),
  conversationCount: document.querySelectorAll('.conversation-node').length,
  actorCount: document.querySelectorAll('.agent-identity').length,
  viewportScrollable: viewport.scrollWidth > viewport.clientWidth,
  documentOverflow: document.documentElement.scrollWidth > document.documentElement.clientWidth
})
```

Expected: brand `DevMap · Rail View`; title `Repository topology`; conversation and actor counts match visible linked conversations; `viewportScrollable` is true for the wide fixture; `documentOverflow` is false.

Test MAP, READ, and FULL; expand historical conversations; select a worktree and a conversation; drag empty canvas; Shift-wheel; keyboard arrows; native bottom scrollbar. Confirm the sticky worktree identity remains visible and Browser logs contain no errors.

- [ ] **Step 4: Capture and compare the final design**

Save the implementation screenshot as `conversation-panorama-final.jpg`. Create `comparison-conversation-panorama-final.png` with the approved Rail View source and corrected implementation side by side at comparable scale. Inspect the combined image for overlapping text, clipped controls, wrong visual hierarchy, bad spacing, and lost topology context; return to the relevant RED test if any P0/P1/P2 defect remains.

- [ ] **Step 5: Update the Design QA report**

Record the measured viewport widths, conversation/actor counts, interaction results, console result, comparison path, and any remaining P3-only differences in `design-qa.md`.

- [ ] **Step 6: Commit QA evidence**

```powershell
git add -- design-qa.md .superpowers/brainstorm/product-design/conversation-panorama-final.jpg .superpowers/brainstorm/product-design/comparison-conversation-panorama-final.png
git commit -m "[TEST](dock): Verify conversation panorama design"
```

---

### Task 5: Run the release-quality verification gate

**Files:**
- Modify only if a verification failure identifies a specific defect covered by the approved spec.

**Interfaces:**
- Consumes: all implementation and QA commits.
- Produces: fresh evidence that the repository is formatted, warning-free, and fully tested.

- [ ] **Step 1: Stop the viewer before rebuilding on Windows**

Send Ctrl+C to the viewer process and verify it exits so `target/debug/devmap.exe` is not locked.

- [ ] **Step 2: Run formatting and patch checks**

```powershell
cargo fmt --check
git diff --check
```

Expected: both commands exit 0.

- [ ] **Step 3: Run all tests**

```powershell
cargo test --all-targets --all-features
```

Expected: exit 0 with zero failed tests.

- [ ] **Step 4: Run strict linting**

```powershell
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: exit 0 with no warnings.

- [ ] **Step 5: Reopen the verified preview**

Start the same viewer command, open the new loopback URL in the Codex in-app Browser, and confirm the header, topology title, live state, conversation/agent hierarchy, horizontal viewport, and empty console one final time.

- [ ] **Step 6: Inspect final repository state**

```powershell
git status --short --branch
git log -5 --oneline --decorate
```

Expected: only intentionally retained design evidence or previously known changes remain; no unrelated file is staged.
