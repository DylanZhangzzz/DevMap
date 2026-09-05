// Runtime renderer contracts. This small DOM adapter deliberately does not claim
// browser layout, CSS contrast, focus painting, or pixel geometry acceptance.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const topology = require('./fixtures/metro/topology.json');
const root = path.join(__dirname, '..');
const now = Date.now();
const stamp = new Date(now).toISOString();

class Element {
  constructor(tag = 'div') {
    this.tagName = tag; this.children = []; this.dataset = {}; this.attributes = {};
    this.style = { setProperty(key, value) { this[key] = value; } };
    this.listeners = {}; this.hidden = false; this.className = ''; this._text = '';
    this.scrollLeft = 0; this.scrollTop = 0; this.scrollWidth = 2400; this.clientWidth = 360; this.clientHeight = 600;
    this.classList = { add: (s) => { this.className += ' ' + s; }, remove: (s) => { this.className = this.className.replace(s, ''); }, toggle() {} };
  }
  set textContent(s) { this._text = String(s); this.children = []; }
  get textContent() { return this._text + this.children.map(c => c.textContent).join(''); }
  append(...nodes) { for (const node of nodes) { if (node.tagName === '#fragment') this.append(...node.children); else { node.parentNode = this; this.children.push(node); } } }
  appendChild(node) { this.append(node); return node; }
  replaceChildren(...nodes) { this.children = []; this._text = ''; this.append(...nodes); }
  remove() { if (this.parentNode) this.parentNode.children = this.parentNode.children.filter(n => n !== this); }
  setAttribute(k, v) { this.attributes[k] = String(v); }
  getAttribute(k) { return this.attributes[k] ?? null; }
  removeAttribute(k) { delete this.attributes[k]; }
  addEventListener(k, cb) { this.listeners[k] = cb; }
  closest(selector) {
    const candidates = selector.split(',').map(value => value.trim());
    for (let node = this; node; node = node.parentNode) {
      if (candidates.some(candidate => candidate === node.tagName
        || (candidate === '[data-no-pan]' && node.getAttribute('data-no-pan') !== null))) return node;
    }
    return null;
  }
  querySelectorAll(selector) {
    const matches = (n) => selector.startsWith('.') ? n.className.split(' ').includes(selector.slice(1))
      : selector.startsWith('[') ? (() => { const [key, value] = selector.slice(1, -1).split('='); const attr = key.startsWith('data-') ? n.dataset[key.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase())] : n.getAttribute(key); return value === undefined ? attr != null : attr === value.replaceAll('"', ''); })()
      : n.tagName === selector;
    return this.children.flatMap(n => [...(matches(n) ? [n] : []), ...n.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  getBoundingClientRect() {
    const naturalHeight = this.className.includes('worktree-cluster') ? (240 + this.querySelectorAll('.task-node').length * 64) * (this.textScale || 1) : 44;
    // Model only the inline CSS size contract, not browser text/layout pixels.
    return { x: 0, y: 0, width: Number.parseFloat(this.style.width) || 320, height: Math.min(naturalHeight, Number.parseFloat(this.style.maxHeight) || Infinity) };
  }
  scrollTo(options) { this.scrollLeft = options.left ?? this.scrollLeft; this.scrollTop = options.top ?? this.scrollTop; }
  focus() {}
}
function harness({ textScale = 1, mode = 'mcp', nowMs = now } = {}) {
  const ids = new Map(); const events = {};
  const clock = { value: nowMs }; const timers = new Map(); let nextTimer = 1;
  class ClockDate extends Date { static now() { return clock.value; } }
  const document = {
    documentElement: new Element('html'), visibilityState: 'hidden',
    getElementById: (id) => { if (!ids.has(id)) ids.set(id, new Element()); return ids.get(id); },
    createElement: tag => { const node = new Element(tag); node.textScale = textScale; node.focus = () => { document.activeElement = node; }; return node; }, createElementNS: (_, tag) => new Element(tag),
    createDocumentFragment: () => new Element('#fragment'),
    querySelectorAll: selector => [...ids.values()].flatMap(n => n.querySelectorAll(selector)),
    addEventListener() {},
  };
  const messages = [];
  const window = { parent: { postMessage: m => messages.push(m) }, location: { search: '' }, addEventListener: (k, cb) => { events[k] = cb; }, matchMedia: () => ({ matches: true }) };
  if (mode === 'browser') window.parent = window;
  let html = fs.readFileSync(path.join(root, 'assets/dock.html'), 'utf8').replace('/* DEVMAP_METRO_CORE */', fs.readFileSync(path.join(root, 'assets/metro-core.js'), 'utf8'));
  let script = html.slice(html.indexOf('<script>') + 8, html.lastIndexOf('</script>'));
  script = script.replace('if (transport === "mcp") initializeMcp(); else { fetchSnapshot(); connectEvents(); } scheduleAge();', '');
  const last = script.lastIndexOf('})();');
  script = script.slice(0, last) + 'globalThis.renderer = { acceptSnapshot, renderSnapshot, refreshDynamicState: typeof refreshDynamicState === "function" ? refreshDynamicState : () => { throw new Error("refreshDynamicState missing"); }, explorationState: () => ({ selectedWorkspaceId: typeof selectedWorkspaceId === "undefined" ? undefined : selectedWorkspaceId, selectedTaskId: typeof selectedTaskId === "undefined" ? undefined : selectedTaskId, expandedWorkspaces: [...expandedWorkspaces], expandedConversationHistory: [...expandedConversationHistory], viewportPosition: typeof viewportPosition === "undefined" ? undefined : { ...viewportPosition } }) };' + script.slice(last);
  const context = vm.createContext({ document, window, navigator: {}, console, Date: ClockDate,
    setTimeout: cb => { const id = nextTimer++; timers.set(id, cb); return id; }, clearTimeout: id => timers.delete(id),
    requestAnimationFrame: cb => cb(), ResizeObserver: class { observe() {} disconnect() {} } });
  vm.runInContext(script, context);
  return { ...context.renderer, ids, document, messages, advance(ms) { clock.value += ms; }, runTimer(id) { const cb = timers.get(id); timers.delete(id); cb?.(); }, timers };
}
function codexId(value) {
  let hash = 2166136261;
  for (const character of String(value)) { hash ^= character.charCodeAt(0); hash = Math.imul(hash, 16777619) >>> 0; }
  return `01990000-0000-7000-8000-${hash.toString(16).padStart(8, '0')}0000`;
}
function chat(id, title, status = 'active') {
  const threadId = codexId(id);
  return { session_id: 'session-' + id, codex_thread_id: threadId, display_title: title, actor_id: 'codex', host: 'local', host_status: status, route_id: 'thread:' + threadId, status: status === 'active' ? 'working' : status, status_source: 'host_explicit', confidence: 'observed', capture_grade: 'A', last_event_at: stamp, blocker_count: 0, gap_count: 0, capture_incomplete: false, association_source: 'codex_task_cwd' };
}
function snapshot() {
  const lanes = topology.attachments.map((a, i) => ({ worktree_id: a.worktree_id, workspace_path: 'C:/checkouts/' + ['main-folder', 'auth-folder', 'ui-folder', 'api-folder', 'detached-folder', 'auth-copy'][i], is_current: i === 0, branch: i === 0 ? 'main' : 'feature/' + i, head: a.head_oid, relationship: { base_target: 'main', merge_target: 'main', merged: true, ahead: 0, behind: 0, dirty: false, changed_file_count: 0, status_observed: true, fork_point: null }, chats: i === 0 ? [chat('a', '<img onerror=alert(1)> 保留完整任务标题'), chat('b', 'Second exact title'), chat('c', 'Third active title'), chat('d', 'Idle task', 'idle'), chat('e', 'Historical task', 'completed')] : [] }));
  return { schema_version: 'devmap/dock/4', repository_id: 'sha256-' + 'c'.repeat(64), revision: 4, observation_revision: 9, generated_at: stamp, current_worktree_id: lanes[0].worktree_id, development_target: null, integration_branches: [], branch_groups: [{ target_branch: 'main', terminal: false, fork_point: null, lanes }], lanes, current: [], active: [], stale_or_uninstrumented: [], topology: structuredClone(topology.graph), workspace_facts: lanes.map(l => ({ worktree_id: l.worktree_id, head_oid: l.head, detached: false, head_ref_coverage: 'protected', integration: 'included', target_ref: 'refs/heads/main', merge_commit_oid: null, working_state: 'clean', upstream: 'unknown', task_observed_at: stamp, git_observed_at: stamp, writer_evidence: [] })), task_observation: { observed_at: stamp, complete: true }, counts: { workspaces: lanes.length, tasks: 5 }, task_inventory_synced_at: stamp, warnings: [], truncated: false };
}

// Reusable synthetic snapshot builder for the coordinated browser acceptance batch.
module.exports = { snapshot, chat };
if (require.main === module) {
test('first view focuses the current workspace at readable size; full map is explicit', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const map = ui.ids.get('relationship-map'), world = ui.ids.get('topology-world'), viewport = ui.ids.get('topology-viewport');
  assert.equal(Number(map.dataset.scale), 1);
  assert.equal(ui.explorationState().selectedWorkspaceId, value.current_worktree_id);
  ui.ids.get('zoom-fit').listeners.click();
  const scale = Number(map.dataset.scale);
  assert.ok(scale > 0 && scale < .6, 'full map explicitly zooms out');
  assert.ok(parseFloat(world.style.width) <= viewport.clientWidth);
  assert.ok(parseFloat(world.style.height) >= 100, 'named locations may extend vertically');
  const overview = ui.ids.get('overview-workspaces');
  assert.equal(overview.hidden, false);
  assert.equal(overview.querySelectorAll('.overview-workspace').length, value.lanes.length);
  assert.ok(overview.textContent.includes('main-folder'));
  assert.ok(overview.textContent.includes('5 tasks'));
  assert.equal(overview.style.transform, undefined, 'readable summaries must not share the map transform');
});

test('overview workspace activation zooms to that exact checkout without opening a task', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  const id = value.lanes[4].worktree_id;
  const overview = ui.ids.get('overview-workspaces'); assert.ok(overview, 'readable overview exists');
  const summary = overview.querySelectorAll('.overview-workspace').find(node => node.dataset.worktreeId === id);
  assert.ok(summary, 'each checkout must have its own readable target');
  summary.listeners.click();
  assert.equal(Number(ui.ids.get('relationship-map').dataset.scale), 1);
  assert.equal(ui.explorationState().selectedWorkspaceId, id);
  assert.equal(ui.ids.get('overview-workspaces').hidden, true);
  assert.equal(ui.messages.filter(message => message.method === 'ui/message').length, 0);
  assert.equal(ui.document.activeElement.dataset.objectId, 'workspace:' + id);
  assert.equal(ui.document.activeElement.getAttribute('aria-current'), 'true', 'drill-down selection is visible before another snapshot arrives');
});

test('zoom preserves the center world point and survives observation, age, and geometry refresh', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  assert.ok(ui.ids.get('zoom-reset'), '100% control exists');
  assert.equal(typeof ui.ids.get('zoom-reset').listeners.click, 'function');
  ui.ids.get('zoom-reset').listeners.click();
  const viewport = ui.ids.get('topology-viewport'), map = ui.ids.get('relationship-map');
  viewport.scrollLeft = 1000; viewport.scrollTop = 600; viewport.listeners.scroll();
  const point = [(viewport.scrollLeft + viewport.clientWidth / 2 - 16), (viewport.scrollTop + viewport.clientHeight / 2 - 16)];
  ui.ids.get('zoom-out').listeners.click();
  const scale = Number(map.dataset.scale);
  assert.ok(scale > .6 && scale < 1);
  assert.ok(Math.abs((viewport.scrollLeft + viewport.clientWidth / 2 - 16) / scale - point[0]) < .001);
  assert.ok(Math.abs((viewport.scrollTop + viewport.clientHeight / 2 - 16) / scale - point[1]) < .001);
  const saved = ui.explorationState().viewportPosition;
  const changed = structuredClone(value); changed.observation_revision++; changed.revision++;
  changed.lanes[1].chats.push(chat('zoom-extra', 'Additional Agent')); changed.branch_groups[0].lanes = changed.lanes;
  assert.equal(ui.acceptSnapshot(changed), true); ui.advance(121000); ui.refreshDynamicState();
  assert.equal(Number(map.dataset.scale), scale);
  assert.deepEqual(ui.explorationState().viewportPosition, saved);
  ui.ids.get('zoom-fit').listeners.click();
  assert.ok(Number(map.dataset.scale) < .6);
  assert.equal(viewport.scrollLeft, 0); assert.equal(viewport.scrollTop, 0);
});

test('overview preserves dirty, not-included and unknown activity independently at every zoom', () => {
  const ui = harness(), value = snapshot();
  value.workspace_facts[0].working_state = 'dirty'; value.workspace_facts[0].integration = 'ahead';
  value.task_observation.complete = false;
  ui.acceptSnapshot(value);
  const overview = ui.ids.get('overview-workspaces');
  assert.ok(overview, 'risk summary exists');
  assert.ok(overview.textContent.includes('Uncommitted changes'));
  assert.ok(overview.textContent.includes('Commits not included'));
  assert.ok(overview.textContent.includes('Task activity unknown'));
  assert.ok(overview.textContent.includes('Last observed'));
  ui.advance(180000); ui.refreshDynamicState();
  assert.ok(overview.textContent.includes('Task activity unknown'));
});

test('map zoom keys are scoped to the canvas and preserve browser zoom shortcuts', () => {
  const ui = harness(); ui.acceptSnapshot(snapshot());
  const viewport = ui.ids.get('topology-viewport'), map = ui.ids.get('relationship-map');
  const before = Number(map.dataset.scale); assert.ok(before > 0);
  viewport.listeners.keydown({ target: viewport, key: '+', preventDefault() {} });
  assert.ok(Number(map.dataset.scale) > before);
  const after = Number(map.dataset.scale);
  viewport.listeners.keydown({ target: viewport, key: '-', ctrlKey: true, preventDefault() { throw new Error('browser shortcut intercepted'); } });
  assert.equal(Number(map.dataset.scale), after);
  const child = new Element('button');
  viewport.listeners.keydown({ target: child, key: '-', preventDefault() { throw new Error('nested input intercepted'); } });
  assert.equal(Number(map.dataset.scale), after);
});

test('overview marks workspace HEADs, groups shared HEADs, and preserves readable stroke scaling', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  ui.ids.get('zoom-fit').listeners.click();
  const map = ui.ids.get('relationship-map');
  assert.equal(Number(map.style['--map-scale']), Number(map.dataset.scale));
  const markers = ui.ids.get('overview-markers'); assert.ok(markers, 'map locations remain visible');
  const stops = markers.querySelectorAll('.overview-marker'); assert.ok(stops.length > 0);
  for (const stop of stops) {
    assert.ok(value.workspace_facts.some(fact => fact.head_oid === stop.dataset.headOid));
    assert.ok(stop.getAttribute('aria-label').includes('workspace'));
    assert.ok(stop.textContent.includes('folder') || stop.textContent.includes('workspaces'));
    assert.ok(!/^[\d· !]+$/.test(stop.textContent), 'locations must identify themselves without a number key');
  }
  const shared = stops.find(stop => Number(stop.dataset.workspaceCount) > 1);
  assert.ok(shared, 'shared HEAD is one location, not an invented new commit');
  shared.listeners.click();
  assert.ok(Number(map.dataset.scale) < .6, 'expand shared workspaces without losing the overview');
  ui.acceptSnapshot({ ...value, observation_revision: value.observation_revision + 1 });
  const choices = ui.ids.get('selection-details').querySelectorAll('.workspace-choice');
  assert.equal(choices.length, Number(shared.dataset.workspaceCount));
  choices[1].listeners.click();
  assert.equal(Number(map.dataset.scale), 1);
  assert.equal(ui.explorationState().selectedWorkspaceId, choices[1].dataset.worktreeId);
  assert.equal(ui.messages.filter(message => message.method === 'ui/message').length, 0);
});

test('overview marker focus survives refresh and a single location selects its exact workspace', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  ui.ids.get('zoom-fit').listeners.click();
  const markers = ui.ids.get('overview-markers');
  const stop = markers.querySelectorAll('.overview-marker').find(n => n.dataset.workspaceCount === '1');
  assert.ok(stop); stop.focus();
  ui.acceptSnapshot({...value, observation_revision: 10});
  assert.notEqual(ui.document.activeElement, stop);
  assert.equal(ui.document.activeElement.dataset.headOid, stop.dataset.headOid);
  ui.document.activeElement.listeners.click();
  assert.equal(ui.explorationState().selectedWorkspaceId, value.workspace_facts.find(f => f.head_oid === stop.dataset.headOid).worktree_id);
});

test('zoom is bounded, invalid snapshots preserve it, and changing repositories focuses current workspace', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  const map = ui.ids.get('relationship-map');
  for (let i=0;i<40;i++) ui.ids.get('zoom-in').listeners.click();
  assert.equal(Number(map.dataset.scale), 2); assert.equal(ui.ids.get('zoom-in').disabled, true);
  assert.equal(ui.acceptSnapshot({...value, schema_version: 'invalid'}), false);
  assert.equal(Number(map.dataset.scale), 2);
  const changed = structuredClone(value); changed.repository_id = 'sha256-' + 'd'.repeat(64);
  changed.revision++; changed.observation_revision++;
  assert.equal(ui.acceptSnapshot(changed), true); assert.equal(Number(map.dataset.scale), 1);
  for (let i=0;i<40;i++) ui.ids.get('zoom-out').listeners.click();
  assert.ok(Number(map.dataset.scale)>0); assert.equal(ui.ids.get('zoom-out').disabled,true);
});

test('selected workspace details refresh facts and HEAD without selection messages', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  ui.ids.get('relationship-map').querySelector('.worktree-identity').listeners.click();
  const messages = ui.messages.length, before = ui.explorationState().viewportPosition;
  const changed = structuredClone(value); changed.revision++; changed.observation_revision++;
  changed.workspace_facts[0].working_state = 'dirty';
  changed.workspace_facts[0].head_oid = changed.lanes[1].head;
  changed.lanes[0].head = changed.lanes[1].head;
  changed.lanes[0].relationship.changed_file_count = 3;
  assert.equal(ui.acceptSnapshot(changed), true);
  assert.match(ui.ids.get('selection-details').textContent, /Working treedirty/);
  assert.ok(ui.ids.get('selection-details').textContent.includes(changed.lanes[1].head));
  assert.equal(ui.messages.length, messages);
  assert.deepEqual(ui.explorationState().viewportPosition, before);
  ui.advance(120001); ui.refreshDynamicState();
  assert.match(ui.ids.get('selection-details').textContent, /stale/);
});

test('selected task refresh follows its verified identity through rename and relocation', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  ui.ids.get('relationship-map').querySelector('.task-node').listeners.click();
  const messages = ui.messages.length;
  const changed = structuredClone(value); changed.revision++; changed.observation_revision++;
  const moved = changed.lanes[0].chats.shift(); moved.display_title = 'Renamed after moving';
  changed.lanes[1].chats.push(moved);
  assert.equal(ui.acceptSnapshot(changed), true);
  assert.match(ui.ids.get('selection-details').textContent, /Renamed after moving/);
  assert.ok(ui.ids.get('selection-details').textContent.includes(changed.lanes[1].workspace_path));
  assert.equal(ui.explorationState().selectedWorkspaceId, changed.lanes[1].worktree_id);
  assert.equal(ui.messages.length, messages);
  assert.match(ui.ids.get('interaction-feedback').textContent, /verify the destination/);
});

test('age tick preserves focus on the exact non-first inspector endpoint without navigation side effects', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  const selectedOid = '0000000000000000000000000000000000000002';
  ui.ids.get('relationship-map').querySelector(`[data-commit-oid="${selectedOid}"]`).listeners.click();
  const details = ui.ids.get('selection-details'), endpoints = details.querySelectorAll('.edge-navigation');
  const focused = endpoints.find(node => node.dataset.endpointOid === '000000000000000000000000000000000000000b');
  assert.ok(focused && focused !== endpoints[0], 'fixture must focus a later endpoint');
  const identity = { oid: focused.dataset.endpointOid, action: focused.dataset.navigationAction };
  const viewport = ui.ids.get('topology-viewport'); viewport.scrollLeft = 321; viewport.scrollTop = 45; viewport.listeners.scroll();
  const before = ui.explorationState().viewportPosition, messages = ui.messages.length;
  focused.focus(); details.scrollTop = 40;

  ui.advance(1000); ui.refreshDynamicState();

  const restored = details.querySelectorAll('.edge-navigation').find(node => node.dataset.endpointOid === identity.oid && node.dataset.navigationAction === identity.action);
  assert.notEqual(ui.document.activeElement, focused, 'focus must move off the detached old endpoint');
  assert.equal(ui.document.activeElement, restored);
  assert.equal(details.scrollTop, 40);
  assert.deepEqual(ui.explorationState().viewportPosition, before);
  assert.equal(ui.messages.length, messages);
});

test('accepted refresh preserves an exact inspector endpoint and does not substitute one after it disappears', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  const selectedOid = '0000000000000000000000000000000000000002';
  const endpointOid = '000000000000000000000000000000000000000b';
  ui.ids.get('relationship-map').querySelector(`[data-commit-oid="${selectedOid}"]`).listeners.click();
  const details = ui.ids.get('selection-details');
  let focused = details.querySelectorAll('.edge-navigation').find(node => node.dataset.endpointOid === endpointOid);
  const viewport = ui.ids.get('topology-viewport'); viewport.scrollLeft = 321; viewport.scrollTop = 45; viewport.listeners.scroll();
  const before = ui.explorationState().viewportPosition, messages = ui.messages.length;
  const action = focused.dataset.navigationAction; focused.focus();

  const refreshed = structuredClone(value); refreshed.observation_revision++;
  assert.equal(ui.acceptSnapshot(refreshed), true);
  focused = details.querySelectorAll('.edge-navigation').find(node => node.dataset.endpointOid === endpointOid && node.dataset.navigationAction === action);
  assert.equal(ui.document.activeElement, focused);
  assert.deepEqual(ui.explorationState().viewportPosition, before);
  assert.equal(ui.messages.length, messages);

  const disappeared = structuredClone(refreshed); disappeared.revision++; disappeared.observation_revision++;
  disappeared.topology.edges = disappeared.topology.edges.filter(edge => edge.from_oid !== endpointOid && edge.to_oid !== endpointOid);
  disappeared.topology.refs = disappeared.topology.refs.filter(reference => reference.oid !== endpointOid);
  disappeared.topology.commits = disappeared.topology.commits.filter(commit => commit.oid !== endpointOid);
  assert.equal(ui.acceptSnapshot(disappeared), true);
  const remaining = details.querySelectorAll('.edge-navigation');
  assert.ok(remaining.length > 0, 'fixture must retain another endpoint');
  assert.ok(!remaining.includes(ui.document.activeElement), 'a different endpoint must not inherit focus');
  assert.equal(ui.messages.length, messages);
});

test('refreshing open details preserves keyboard focus on the selected inspector action', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  ui.ids.get('relationship-map').querySelector('.worktree-identity').listeners.click();
  const details = ui.ids.get('selection-details'), copy = details.querySelector('.copy-hash');
  copy.focus(); details.scrollTop = 40;
  ui.advance(1000); ui.refreshDynamicState();
  assert.notEqual(ui.document.activeElement, copy, 'focus must follow the rebuilt inspector control');
  assert.equal(ui.document.activeElement, details.querySelector('.copy-hash'));
  assert.equal(details.scrollTop, 40);
});

test('verified task rows expose resting Open task affordance and unverified rows say Inspect only', () => {
  const ui = harness(), value = snapshot();
  value.lanes[0].chats = value.lanes[0].chats.slice(0, 2);
  value.lanes[0].chats[1].codex_thread_id = null;
  ui.acceptSnapshot(value);
  const rows = ui.ids.get('relationship-map').querySelectorAll('.task-node');
  assert.equal(rows[0].querySelector('.task-onward')?.textContent, 'Open task');
  assert.equal(rows[1].querySelector('.task-onward')?.textContent, 'Inspect only');
  assert.equal(rows[1].dataset.navigable, 'false');
  assert.equal(rows[0].querySelector('.task-title').textContent, value.lanes[0].chats[0].display_title);
});

test('budget-reduced task detail stays explicitly partial beside the named workspace and risk', () => {
  const ui = harness(), value = snapshot();
  value.branch_groups = []; value.truncated = true; value.task_observation.complete = false;
  value.lanes[0].chats = [];
  value.workspace_facts[0].working_state = 'dirty';
  value.warnings.push({ code: 'workspace_detail_partial', subject_id: value.lanes[0].worktree_id });
  assert.equal(ui.acceptSnapshot(value), true);
  const card = ui.ids.get('relationship-map').querySelector('[data-worktree-id]');
  assert.match(card.textContent, /Partial task detail/);
  assert.ok(!card.textContent.includes('No linked task observed'));
  assert.match(card.textContent, /Uncommitted changes/);
  assert.match(card.querySelector('.worktree-identity').getAttribute('aria-label'), /C:\/checkouts\/main-folder/);
  assert.match(card.textContent, /Task activity unknown/);
});

test('v4 renders actual graph, measured workspaces, exact names, and keeps invalid snapshots atomic', () => {
  const ui = harness(), value = snapshot();
  assert.equal(ui.acceptSnapshot(value), true, 'v4 must be accepted');
  const surface = ui.ids.get('relationship-map');
  assert.equal(surface.querySelectorAll('[data-commit-oid]').length, 12);
  assert.equal(surface.querySelectorAll('[data-worktree-id]').length, 6);
  assert.equal(surface.querySelectorAll('[data-edge-id]').length, 12);
  assert.ok(surface.querySelector('.worktree-identity').getAttribute('aria-label').includes('checkouts/main-folder'));
  assert.ok(surface.textContent.includes('<img onerror=alert(1)> 保留完整任务标题'));
  assert.equal(surface.querySelectorAll('img').length, 0);
  assert.equal(surface.querySelector('.task-count-summary').textContent, '3 active tasks · 1 idle');
  assert.equal(surface.querySelectorAll('.task-node').length, 2);
  const layout = surface.metroLayout;
  const card = surface.querySelectorAll('[data-worktree-id]')[0];
  assert.equal(layout.attachments.find(a => a.worktree_id === card.dataset.worktreeId).height, card.getBoundingClientRect().height);
  const children = [...surface.children];
  const invalid = structuredClone(value); invalid.revision++; invalid.topology.edges[0].to_oid = 'f'.repeat(40);
  assert.equal(ui.acceptSnapshot(invalid), false);
  assert.deepEqual(surface.children, children, 'invalid input keeps the exact last good DOM');
  assert.equal(ui.acceptSnapshot({ ...value, schema_version: 'devmap/dock/99' }), false);
});

test('v3 has an explicit limited view with workspaces and no invented commit connections', () => {
  const ui = harness(), value = snapshot(); value.schema_version = 'devmap/dock/3';
  for (const key of ['topology', 'workspace_facts', 'task_observation', 'counts', 'observation_revision']) delete value[key];
  assert.equal(ui.acceptSnapshot(value), true);
  assert.ok(ui.ids.get('coverage')?.textContent.includes('Limited view'));
  assert.equal(ui.ids.get('relationship-map').querySelectorAll('[data-commit-oid]').length, 0);
  assert.equal(ui.ids.get('relationship-map').querySelectorAll('[data-edge-id]').length, 0);
  assert.ok(ui.ids.get('relationship-map').textContent.includes('保留完整任务标题'));
});

test('observation-only envelopes update independent facts without relaying out geometry', () => {
  const ui = harness(), value = snapshot();
  value.workspace_facts[0].working_state = 'dirty';
  assert.equal(ui.acceptSnapshot(value), true);
  const surface = ui.ids.get('relationship-map'), layout = surface.metroLayout;
  assert.ok(surface.textContent.includes('Uncommitted changes'));
  assert.ok(surface.textContent.includes('Commits included'));
  assert.ok(surface.textContent.includes('Publication unknown'));
  const stale = structuredClone(value); stale.observation_revision++; stale.task_observation.observed_at = new Date(now - 300000).toISOString();
  assert.equal(ui.acceptSnapshot(stale), true);
  assert.equal(surface.metroLayout, layout);
  assert.ok(surface.textContent.includes('Task activity unknown'));
  assert.ok(!surface.textContent.includes('No active task observed'));
});

test('unborn workspace retains tasks and dirty facts without a fabricated zero-OID station', () => {
  const ui = harness(), value = snapshot();
  value.topology = { commits: [], refs: [], edges: [], boundaries: [], complete: true };
  value.lanes = value.lanes.slice(0, 1); value.branch_groups[0].lanes = value.lanes;
  value.lanes[0].head = '0'.repeat(40); value.workspace_facts = value.workspace_facts.slice(0, 1);
  Object.assign(value.workspace_facts[0], { head_oid: '0'.repeat(40), working_state: 'dirty', integration: 'terminal' });
  value.counts.workspaces = 1;
  assert.equal(ui.acceptSnapshot(value), true);
  const surface = ui.ids.get('relationship-map');
  assert.equal(surface.querySelectorAll('[data-commit-oid]').length, 0);
  assert.equal(surface.querySelectorAll('[data-worktree-id]').length, 1);
  assert.ok(surface.textContent.includes('No commits yet'));
  assert.ok(surface.textContent.includes('Uncommitted changes'));
  assert.ok(surface.textContent.includes('保留完整任务标题'));
});

test('task expansion reallocates measured cards and history remains explicitly disclosed', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const surface = ui.ids.get('relationship-map');
  const initial = surface.metroLayout.attachments.find(a => a.worktree_id === value.current_worktree_id);
  surface.querySelectorAll('.task-disclosure').find(n => n.textContent.includes('Show all active')).listeners.click();
  assert.equal(surface.querySelectorAll('.task-node').length, 4);
  assert.ok(surface.textContent.includes('Third active title'));
  assert.ok(surface.textContent.includes('Idle task'));
  assert.ok(!surface.textContent.includes('Historical task'));
  const expanded = surface.metroLayout.attachments.find(a => a.worktree_id === value.current_worktree_id);
  assert.equal(expanded.height - initial.height, 128, 'two newly visible rows expand the measured adapter card by 128px');
  surface.querySelectorAll('.historical-conversations')[0].listeners.click();
  assert.equal(surface.querySelectorAll('.task-node').length, 5);
  assert.ok(surface.textContent.includes('Historical task'));
});

test('a two-name default preview is followed by an explicit active count', () => {
  const ui = harness(), value = snapshot();
  value.lanes[0].chats = value.lanes[0].chats.slice(0, 2);
  value.branch_groups[0].lanes = value.lanes; value.counts.tasks = 2;
  assert.equal(ui.acceptSnapshot(value), true);
  const card = ui.ids.get('relationship-map').querySelectorAll('[data-worktree-id]')[0];
  assert.equal(card.querySelectorAll('.task-node').length, 2);
  const summary = card.querySelector('.task-count-summary');
  assert.ok(summary, 'the visible names need a following active-count summary');
  assert.equal(summary.textContent, '2 active tasks');
});

test('SVG routes use station centers and each crossing masks only its underpassing edge', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const surface = ui.ids.get('relationship-map'), layout = surface.metroLayout;
  const nodes = new Map(surface.querySelectorAll('[data-commit-oid]').map(n => [n.dataset.commitOid, n]));
  const edges = surface.querySelectorAll('[data-edge-id]');
  assert.ok(layout.crossings.length > 0, 'fixture must exercise gaps');
  for (const group of edges) {
    const path = group.children.at(-1), numbers = path.getAttribute('d').match(/-?\d+(?:\.\d+)?/g).map(Number);
    const from = nodes.get(group.dataset.fromOid), to = nodes.get(group.dataset.toOid);
    assert.deepEqual(numbers.slice(0, 2), [parseFloat(from.style.left), parseFloat(from.style.top)]);
    assert.deepEqual(numbers.slice(-2), [parseFloat(to.style.left), parseFloat(to.style.top)]);
    const edge = layout.edges.find(e => e.id === group.dataset.edgeId);
    if (edge.gaps.length) {
      assert.match(group.getAttribute('mask'), /^url\(#metro-gap-\d+\)$/);
      const id = group.getAttribute('mask').slice(5, -1);
      const mask = surface.querySelectorAll('mask').find(m => m.getAttribute('id') === id);
      assert.equal(mask.children.filter(p => p.getAttribute('stroke') === 'black').length, edge.gaps.length);
    } else assert.equal(group.getAttribute('mask'), null);
  }
});

test('a nonzero boundary attachment is history-limited, never an unborn workspace', () => {
  const ui = harness(), value = snapshot(); const oid = value.lanes[0].head;
  value.topology = { commits: [], refs: [], edges: [], boundaries: [{ id: 'missing-head', oid, reason: 'history_limit' }], complete: false };
  value.lanes = value.lanes.slice(0, 1); value.branch_groups[0].lanes = value.lanes; value.workspace_facts = value.workspace_facts.slice(0, 1); value.counts.workspaces = 1;
  assert.equal(ui.acceptSnapshot(value), true);
  const surface = ui.ids.get('relationship-map');
  assert.equal(surface.querySelectorAll('[data-commit-oid]').length, 1);
  assert.ok(surface.textContent.includes('History limit reached'));
  assert.ok(!surface.textContent.includes('No commits yet'));
});

test('task activation uses only verified task IDs and heartbeat cannot erase feedback', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const node = ui.ids.get('relationship-map').querySelectorAll('.task-node')[0];
  node.listeners.click();
  const message = ui.messages.find(m => m.method === 'ui/message');
  assert.equal(message.params.content[0].text, `Open the local Codex task with id ${codexId('a')}.`);
  const feedback = ui.ids.get('interaction-feedback').textContent;
  assert.equal(ui.acceptSnapshot({ ...value, observation_revision: value.observation_revision + 1 }), true);
  assert.equal(ui.ids.get('interaction-feedback').textContent, feedback);
  assert.ok(!JSON.stringify(message).includes('保留完整任务标题'));
});

test('browser task drill-down exposes a verified user-activated deep link without claiming arrival', () => {
  const ui = harness({ mode: 'browser' }), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const node = ui.ids.get('relationship-map').querySelectorAll('.task-node')[0];
  assert.equal(node.tagName, 'a');
  assert.equal(node.getAttribute('href'), `codex://threads/${codexId('a')}`);
  assert.equal(node.getAttribute('target'), '_blank');
  let prevented = false;
  node.listeners.click({ preventDefault() { prevented = true; } });
  assert.equal(prevented, false, 'the installed host must receive a normal user-activated link');
  assert.match(ui.ids.get('interaction-feedback').textContent, /request sent.*verify the destination/i);
  assert.ok(ui.ids.get('selection-details').textContent.includes(codexId('a')), 'inspector keeps the exact-ID fallback');
});

test('unsupported task records remain inspectable and MCP requests recover on timeout', () => {
  const browser = harness({ mode: 'browser' }), unsupported = snapshot();
  unsupported.lanes[0].chats = [unsupported.lanes[0].chats[0]];
  unsupported.lanes[0].chats[0].codex_thread_id = null;
  unsupported.branch_groups[0].lanes = unsupported.lanes;
  assert.equal(browser.acceptSnapshot(unsupported), true);
  const inspect = browser.ids.get('relationship-map').querySelectorAll('.task-node').find(node => node.textContent.includes('保留完整任务标题'));
  assert.equal(inspect.tagName, 'button');
  inspect.listeners.click({ preventDefault() {} });
  assert.match(browser.ids.get('interaction-feedback').textContent, /no verified Codex task link/i);

  const mcp = harness(), value = snapshot(); assert.equal(mcp.acceptSnapshot(value), true);
  const node = mcp.ids.get('relationship-map').querySelectorAll('.task-node')[0];
  node.listeners.click({ preventDefault() {} });
  assert.equal(node.getAttribute('aria-busy'), 'true');
  const timeoutId = [...mcp.timers.keys()].at(-1); mcp.runTimer(timeoutId);
  assert.equal(node.getAttribute('aria-busy'), null);
  assert.match(mcp.ids.get('interaction-feedback').textContent, /could not be opened/i);
});

test('exploration state survives observation refresh and reports selected objects that disappear', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  let surface = ui.ids.get('relationship-map');
  surface.querySelectorAll('.task-disclosure').find(node => node.textContent.includes('Show all active')).listeners.click();
  surface = ui.ids.get('relationship-map');
  const selected = surface.querySelectorAll('.task-node').find(node => node.textContent.includes('Third active title'));
  selected.listeners.click({ preventDefault() {} });
  const viewport = ui.ids.get('topology-viewport'); viewport.scrollLeft = 321; viewport.scrollTop = 45;
  assert.equal(typeof viewport.listeners.scroll, 'function', 'viewport state must be captured independently of DOM replacement');
  viewport.listeners.scroll();
  assert.equal(ui.acceptSnapshot({ ...value, observation_revision: 10 }), true);
  const state = ui.explorationState();
  assert.equal(state.selectedWorkspaceId, value.current_worktree_id);
  assert.equal(state.selectedTaskId, codexId('c'));
  assert.ok(state.expandedWorkspaces.includes(value.current_worktree_id));
  assert.equal(state.viewportPosition.left, 321); assert.equal(state.viewportPosition.top, 45);

  const withoutTask = structuredClone(value); withoutTask.observation_revision = 11;
  withoutTask.lanes[0].chats = withoutTask.lanes[0].chats.filter(task => task.codex_thread_id !== codexId('c'));
  withoutTask.branch_groups[0].lanes = withoutTask.lanes;
  assert.equal(ui.acceptSnapshot(withoutTask), true);
  assert.match(ui.ids.get('interaction-feedback').textContent, /Selected task .* is no longer available/);

  const currentWorkspace = ui.ids.get('relationship-map').querySelectorAll('[data-worktree-id]')
    .find(card => card.dataset.worktreeId === value.current_worktree_id)
    .querySelector('.worktree-identity');
  currentWorkspace.listeners.click();

  const withoutWorkspace = structuredClone(withoutTask); withoutWorkspace.observation_revision = 12;
  withoutWorkspace.lanes = withoutWorkspace.lanes.filter(lane => lane.worktree_id !== value.current_worktree_id);
  withoutWorkspace.branch_groups[0].lanes = withoutWorkspace.lanes;
  withoutWorkspace.workspace_facts = withoutWorkspace.workspace_facts.filter(fact => fact.worktree_id !== value.current_worktree_id);
  withoutWorkspace.counts.workspaces--;
  assert.equal(ui.acceptSnapshot(withoutWorkspace), true);
  assert.match(ui.ids.get('interaction-feedback').textContent, /Selected workspace .* is no longer available/);
});

test('dismissing task details clears retained selection before that task disappears', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const task = ui.ids.get('relationship-map').querySelectorAll('.task-node')[0];
  task.listeners.click({ preventDefault() {} });
  ui.ids.get('selection-details').querySelector('.dismiss-details').listeners.click();
  const updated = structuredClone(value); updated.observation_revision++;
  updated.lanes[0].chats = updated.lanes[0].chats.filter(chat => chat.codex_thread_id !== task.dataset.objectId.slice(5));
  updated.branch_groups[0].lanes = updated.lanes;
  assert.equal(ui.acceptSnapshot(updated), true);
  assert.equal(ui.ids.get('selection-details').hidden, true);
  assert.ok(!ui.ids.get('interaction-feedback').textContent.includes('no longer available'));
});

test('selecting a commit supersedes retained task selection before task removal', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const surface = ui.ids.get('relationship-map'), task = surface.querySelectorAll('.task-node')[0];
  task.listeners.click({ preventDefault() {} });
  const station = surface.querySelectorAll('[data-commit-oid]')[0]; station.listeners.click();
  const commitTitle = ui.ids.get('selection-details').querySelector('h2').textContent;
  const updated = structuredClone(value); updated.observation_revision++;
  updated.lanes[0].chats = updated.lanes[0].chats.filter(chat => chat.codex_thread_id !== task.dataset.objectId.slice(5));
  updated.branch_groups[0].lanes = updated.lanes;
  assert.equal(ui.acceptSnapshot(updated), true);
  assert.equal(ui.ids.get('selection-details').querySelector('h2').textContent, commitTitle);
  assert.ok(!ui.ids.get('interaction-feedback').textContent.includes('no longer available'));
});

test('pan controls and locating move the topology while nested workspace controls retain native input', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  const viewport = ui.ids.get('topology-viewport'); viewport.scrollLeft = 500;
  assert.ok(ui.ids.get('pan-left'), 'a labeled pan control must exist');
  ui.ids.get('pan-left').listeners.click();
  assert.ok(viewport.scrollLeft < 500);
  const afterButton = viewport.scrollLeft;
  let prevented = false;
  viewport.listeners.keydown({ target: viewport, key: 'End', preventDefault() { prevented = true; } });
  assert.equal(prevented, true); assert.equal(viewport.scrollLeft, viewport.scrollWidth);
  const card = ui.ids.get('relationship-map').querySelectorAll('[data-worktree-id]')[0];
  viewport.listeners.keydown({ target: card, key: 'Home', preventDefault() { throw new Error('nested input was captured'); } });
  assert.equal(viewport.scrollLeft, viewport.scrollWidth);
  const task = card.querySelectorAll('.task-node')[0];
  viewport.listeners.pointerdown({ button: 0, target: task, pointerId: 1, clientX: 100 });
  viewport.listeners.pointermove({ pointerId: 1, clientX: 0 });
  assert.equal(viewport.scrollLeft, viewport.scrollWidth, 'clickable Agent rows cannot start background dragging');
  viewport.scrollLeft = 300; let wheelPrevented = false;
  viewport.listeners.wheel({ target: card, shiftKey: true, deltaY: 60, preventDefault() { wheelPrevented = true; } });
  assert.equal(wheelPrevented, false, 'nested workspace Shift+wheel must remain native');
  assert.equal(viewport.scrollLeft, 300, 'nested workspace Shift+wheel must not pan the canvas');
  assert.ok(afterButton >= 0);
});

test('current and risk drill-down keep the station, stem and full workspace preview together', () => {
  const ui = harness(), value = snapshot();
  value.workspace_facts[1].detached = true; value.workspace_facts[1].head_ref_coverage = 'unprotected';
  assert.equal(ui.acceptSnapshot(value), true);
  const viewport = ui.ids.get('topology-viewport'), layout = ui.ids.get('relationship-map').metroLayout;
  const assertJoint = id => {
    const card = layout.attachments.find(a => a.worktree_id === id);
    const head = layout.nodes.find(n => n.id === card.head_oid);
    assert.ok(16 + head.y - 10 >= viewport.scrollTop, 'HEAD must be visible above its Workspace');
    assert.ok(16 + head.x - 10 >= viewport.scrollLeft, 'station must fit beside the workspace');
    assert.ok(16 + card.x + card.width <= viewport.scrollLeft + viewport.clientWidth, 'full title/preview width must fit');
    assert.ok(16 + card.y + card.height <= viewport.scrollTop + viewport.clientHeight, 'default preview must fit with its HEAD');
    assert.ok(card.y - head.y <= 32, 'workspace precedes verbose reference metadata');
    assert.ok(card.stem.points.at(-1).y <= card.y + 32, 'stem attaches to workspace identity, not below the preview');
  };
  ui.ids.get('locate-current').listeners.click();
  assertJoint(value.current_worktree_id);
  const currentHead = layout.nodes.find(node => node.id === value.workspace_facts[0].head_oid);
  assert.ok(currentHead.x >= viewport.scrollLeft && currentHead.x <= viewport.scrollLeft + viewport.clientWidth);
  const risk = ui.ids.get('attention-summary').querySelectorAll('.attention-link').find(node => node.textContent.includes('auth-folder'));
  assert.ok(risk); risk.listeners.click();
  assertJoint(value.workspace_facts[1].worktree_id);
  const riskHead = layout.nodes.find(node => node.id === value.workspace_facts[1].head_oid);
  assert.ok(riskHead.x >= viewport.scrollLeft && riskHead.x <= viewport.scrollLeft + viewport.clientWidth);
});

test('attention stays compact while each risky workspace and exact reasons remain discoverable', () => {
  const ui = harness(), value = snapshot();
  value.workspace_facts[1].detached = true; value.workspace_facts[1].head_ref_coverage = 'unprotected';
  ui.acceptSnapshot(value);
  const attention = ui.ids.get('attention-summary');
  assert.equal(attention.children[0].tagName, 'summary');
  assert.match(attention.children[0].textContent, /1 workspace needs attention/);
  const entry = attention.querySelector('.attention-link');
  assert.match(entry.textContent, /auth-folder.*Detached work lacks a stable ref/);
});

test('compact workspace heading retains the full path, branch and stable identity on its accessible action', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  const heading = ui.ids.get('relationship-map').querySelector('.worktree-identity');
  assert.equal(heading.querySelector('.worktree-name').textContent, 'main-folder');
  assert.ok(heading.getAttribute('aria-label').includes('C:/checkouts/main-folder'));
  assert.ok(heading.getAttribute('aria-label').includes(value.current_worktree_id));
  assert.ok(heading.getAttribute('aria-label').includes('main'));
});

test('locating either workspace sharing a HEAD brings its own identity beside the single station', () => {
  const ui = harness(), value = snapshot();
  value.current_worktree_id = value.lanes[5].worktree_id;
  value.workspace_facts[1].detached = true; value.workspace_facts[1].head_ref_coverage = 'unprotected';
  ui.acceptSnapshot(value);
  const check = id => {
    const layout = ui.ids.get('relationship-map').metroLayout;
    const attachment = layout.attachments.find(a => a.worktree_id === id);
    const node = layout.nodes.find(n => n.id === attachment.head_oid);
    assert.equal(attachment.y - node.y, 24, 'requested shared-HEAD workspace goes directly below the unique station');
    assert.equal(layout.nodes.length, 12);
    assert.equal(layout.attachments.length, 6);
  };
  check(value.current_worktree_id);
  ui.ids.get('attention-summary').querySelector('.attention-link').listeners.click();
  check(value.lanes[1].worktree_id);
  value.observation_revision++; ui.acceptSnapshot(value);
  check(value.lanes[1].worktree_id);
});

test('roster states and counts qualify stale, incomplete and missing observations without changing task identity', () => {
  const ui = harness(), value = snapshot();
  ui.acceptSnapshot(value);
  const surface = ui.ids.get('relationship-map');
  assert.equal(surface.querySelector('.conversation-state').textContent, 'ACTIVE');
  for (const observation of [{ observed_at: new Date(now - 180000).toISOString(), complete: true }, { observed_at: stamp, complete: false }, { observed_at: null, complete: false }]) {
    value.observation_revision++; value.task_observation = observation;
    assert.equal(ui.acceptSnapshot(value), true);
    assert.equal(surface.querySelector('.conversation-state').textContent, 'Last observed ACTIVE');
    assert.match(surface.querySelector('.task-count-summary').textContent, /Last observed.*3 active tasks/);
    assert.equal(surface.querySelector('.task-node').dataset.objectId, 'task:' + codexId('a'));
  }
  value.observation_revision++; value.task_observation = { observed_at: stamp, complete: true };
  ui.acceptSnapshot(value); ui.advance(120001); ui.refreshDynamicState();
  assert.equal(surface.querySelector('.conversation-state').textContent, 'Last observed ACTIVE');
  assert.match(surface.querySelector('.task-disclosure').getAttribute('aria-label'), /Last observed.*3 active tasks/);
});

test('valid recovery clears only snapshot rejection feedback and retains independent navigation failure', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  const invalid = structuredClone(value); invalid.topology.edges[0].to_oid = 'f'.repeat(40);
  ui.acceptSnapshot(invalid);
  assert.match(ui.ids.get('snapshot-feedback')?.textContent || '', /Snapshot rejected/, 'validation feedback needs independent ownership');
  const task = ui.ids.get('relationship-map').querySelector('.task-node'); task.listeners.click();
  const timeout = [...ui.timers.keys()].at(-1); ui.runTimer(timeout);
  ui.acceptSnapshot({ ...value, observation_revision: value.observation_revision + 1 });
  assert.equal(ui.ids.get('snapshot-feedback').hidden, true);
  assert.match(ui.ids.get('interaction-feedback').textContent, /could not be opened/);
});

test('incomplete individual task captures qualify their observed state and roster count', () => {
  const ui = harness(), value = snapshot(); value.lanes[0].chats[0].capture_incomplete = true;
  ui.acceptSnapshot(value);
  const surface = ui.ids.get('relationship-map');
  assert.equal(surface.querySelector('.conversation-state').textContent, 'Last observed ACTIVE');
  assert.match(surface.querySelector('.task-count-summary').textContent, /^Last observed/);
  ui.refreshDynamicState();
  assert.match(surface.querySelector('.task-count-summary').textContent, /^Last observed/);
});

test('viewport wayfinding identifies both true endpoints for known offscreen routes and navigates to them', () => {
  const ui = harness(), value = snapshot(); ui.acceptSnapshot(value);
  ui.ids.get('locate-current').listeners.click();
  const surface = ui.ids.get('relationship-map'), viewport = ui.ids.get('topology-viewport');
  const links = ui.ids.get('edge-list')?.querySelectorAll('.edge-destination') || [];
  assert.ok(links.length > 0, 'offscreen connected rails must have visible wayfinding');
  for (const link of links) {
    assert.ok(value.topology.edges.some(e => e.id === link.dataset.edgeId && [e.from_oid, e.to_oid].includes(link.dataset.endpointOid)));
    assert.ok(link.getAttribute('aria-label').includes(link.dataset.endpointOid), 'accessible label preserves exact endpoint identity');
  }
  const target = links[0]; target.listeners.click();
  const node = surface.metroLayout.nodes.find(n => n.id === target.dataset.endpointOid);
  assert.ok(node.x >= viewport.scrollLeft && node.x <= viewport.scrollLeft + viewport.clientWidth);
  assert.ok(node.y >= viewport.scrollTop && node.y <= viewport.scrollTop + viewport.clientHeight);
});

test('wayfinding includes through-routes and leave-reenter paths but excludes fully offscreen routes', () => {
  const ui = harness(); ui.acceptSnapshot(snapshot());
  ui.ids.get('zoom-reset').listeners.click();
  const surface = ui.ids.get('relationship-map'), viewport = ui.ids.get('topology-viewport');
  viewport.scrollLeft = 16; viewport.scrollTop = 16; viewport.clientWidth = 100; viewport.clientHeight = 100;
  const base = surface.metroLayout.edges[0];
  surface.metroLayout = { ...surface.metroLayout, edges: [
    { ...base, id: 'through', points: [{x:-20,y:50},{x:120,y:50}] },
    { ...base, id: 'reenter', points: [{x:20,y:20},{x:120,y:20},{x:120,y:80},{x:20,y:80}] },
    { ...base, id: 'offscreen', points: [{x:120,y:120},{x:180,y:120}] },
  ] };
  viewport.listeners.scroll();
  const visible = [...new Set(ui.ids.get('edge-list').querySelectorAll('.edge-destination').map(node => node.dataset.edgeId))];
  assert.deepEqual(visible, ['through', 'reenter']);
});

test('task freshness ages independently and recently idle dirty work waits for the threshold', () => {
  const ui = harness({ nowMs: now }), value = snapshot();
  value.workspace_facts[0].working_state = 'dirty';
  value.lanes[0].chats = [chat('idle-clock', 'Recently idle task', 'idle')];
  value.lanes[0].chats[0].last_event_at = new Date(now - 5 * 60_000).toISOString();
  value.branch_groups[0].lanes = value.lanes; value.counts.tasks = 1;
  assert.equal(ui.acceptSnapshot(value), true);
  assert.ok(ui.ids.get('task-inventory'), 'task freshness needs its own status element');
  assert.match(ui.ids.get('task-inventory').textContent, /fresh/);
  assert.equal(ui.ids.get('metric-open').textContent, '0');
  assert.ok(ui.ids.get('relationship-map').textContent.includes('Uncommitted changes'));

  ui.advance(120_001); ui.refreshDynamicState();
  assert.match(ui.ids.get('task-inventory').textContent, /stale/);
  assert.ok(ui.ids.get('relationship-map').textContent.includes('Task activity unknown'));

  const threshold = structuredClone(value); threshold.observation_revision++;
  threshold.task_observation.observed_at = new Date(now + 120_001).toISOString();
  threshold.lanes[0].chats[0].last_event_at = new Date(now + 120_001 - 30 * 60_000).toISOString();
  threshold.branch_groups[0].lanes = threshold.lanes;
  assert.equal(ui.acceptSnapshot(threshold), true);
  assert.equal(ui.ids.get('metric-open').textContent, '1');
  assert.ok(ui.ids.get('relationship-map').textContent.includes('linked tasks idle at least 30 minutes'));
});

test('refresh preserves keyboard focus by task identity and selected station viewport offset', () => {
  const ui = harness(), value = snapshot(); assert.equal(ui.acceptSnapshot(value), true);
  ui.ids.get('zoom-reset').listeners.click();
  const surface = ui.ids.get('relationship-map');
  const task = surface.querySelectorAll('.task-node')[0]; task.focus();
  assert.equal(ui.acceptSnapshot({ ...value, observation_revision: 10 }), true);
  assert.notEqual(ui.document.activeElement, task, 'focus must move off the detached old DOM');
  assert.equal(ui.document.activeElement.dataset.objectId, task.dataset.objectId);
  const station = surface.querySelectorAll('[data-commit-oid]')[0]; station.listeners.click();
  const viewport = ui.ids.get('topology-viewport'); viewport.scrollLeft = 40; viewport.scrollTop = 10;
  viewport.listeners.scroll();
  const offset = { x: parseFloat(station.style.left) - 40, y: parseFloat(station.style.top) - 10 };
  const updated = structuredClone(value); updated.revision++; updated.observation_revision = 11;
  const first = updated.topology.commits.find(c => c.parents.length === 0), older = 'f'.repeat(40);
  first.parents.push(older); updated.topology.commits.push({ oid: older, parents: [], authored_at: null, subject: 'Earlier history' });
  updated.topology.edges.push({ id: 'older-edge', from_oid: older, to_oid: first.oid });
  assert.equal(ui.acceptSnapshot(updated), true);
  const moved = surface.querySelectorAll('[data-commit-oid]').find(n => n.dataset.commitOid === station.dataset.commitOid);
  assert.equal(parseFloat(moved.style.left) - viewport.scrollLeft, offset.x);
  assert.equal(parseFloat(moved.style.top) - viewport.scrollTop, offset.y);
});

test('hundreds of active, idle and historical tasks stay reachable inside measured card bounds', () => {
  for (const category of ['active', 'idle', 'completed']) {
    const ui = harness({ textScale: 2 }), value = snapshot(), lane = value.lanes[0];
    lane.chats = Array.from({ length: 400 }, (_, i) => chat('overflow-' + i, '完整任务名称 ' + category + ' ' + i, category));
    value.counts.tasks = 400;
    value.workspace_facts[0].working_state = 'dirty';
    value.workspace_facts[0].detached = true;
    value.workspace_facts[0].head_ref_coverage = 'unprotected';
    assert.ok(Buffer.byteLength(JSON.stringify(value)) < 768 * 1024, 'fixture stays inside the supported payload budget');
    assert.equal(ui.acceptSnapshot(value), true);
    const surface = ui.ids.get('relationship-map');
    const disclosure = surface.querySelectorAll('[data-object-id]').find(n => n.dataset.objectId === (category === 'completed' ? 'history:' : 'tasks:') + lane.worktree_id);
    assert.ok(disclosure);
    assert.doesNotThrow(() => disclosure.listeners.click(), category + ' expansion must not exceed the reviewed geometry dimension cap');
    let card = surface.querySelectorAll('[data-worktree-id]').find(n => n.dataset.worktreeId === lane.worktree_id);
    assert.equal(card.querySelectorAll('.task-node').length, 400, 'every task remains a semantic control');
    assert.equal(card.style.overflowY, 'auto', 'overflow must remain accessible through scrolling');
    assert.equal(card.getAttribute('tabindex'), '0', 'the scroll region must be keyboard reachable');
    assert.ok(card.getAttribute('aria-label').includes('400 linked tasks'));
    assert.ok(card.textContent.includes('完整任务名称 ' + category + ' 399'));
    assert.ok(card.textContent.includes('Detached work lacks a stable ref'));
    assert.equal(ui.ids.get('metric-tasks').textContent, '400');
    assert.equal(ui.ids.get('metric-open').textContent, '1');
    const measured = surface.metroLayout.attachments.find(a => a.worktree_id === lane.worktree_id);
    assert.equal(measured.height, card.getBoundingClientRect().height);
    assert.ok(measured.height <= 16384);
    card.scrollTop = 18000;
    const lastTask = card.querySelectorAll('.task-node').at(-1); lastTask.focus();
    assert.equal(ui.acceptSnapshot({ ...value, observation_revision: value.observation_revision + 1 }), true);
    card = surface.querySelectorAll('[data-worktree-id]').find(n => n.dataset.worktreeId === lane.worktree_id);
    assert.equal(card.scrollTop, 18000, 'refresh preserves position inside the long task list');
    assert.equal(ui.document.activeElement.dataset.objectId, lastTask.dataset.objectId);
  }
});
}
