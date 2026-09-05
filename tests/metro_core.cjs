const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const CORE_PATH = path.join(__dirname, '..', 'assets', 'metro-core.js');
const ATTENTION_FIXTURE = JSON.parse(fs.readFileSync(path.join(__dirname, 'fixtures', 'metro', 'attention.json'), 'utf8'));
const {
  TASK_FRESHNESS_MS,
  IDLE_ATTENTION_MS,
  validateTopology,
  validateSnapshot,
  classifyWorkspace,
  branchColorKey,
  layoutTopology,
} = require(CORE_PATH);

const oid = (character) => character.repeat(40);

const TOPOLOGY = require('./fixtures/metro/topology.json');
const BOUNDARIES = require('./fixtures/metro/boundaries.json');

test('passengers describe chat existence, independent of running state', () => {
  const core = require(CORE_PATH), obs = {complete:true,observedAtMs:1000};
  const tasks = ['completed','idle','notLoaded','waiting'].map((status,i) => ({id:String(i),status,lifecycle:'present'}));
  tasks.push({id:'archived',status:'active',lifecycle:'archived'},{id:'deleted',status:'active',lifecycle:'deleted'});
  const summary = core.summarizePassengers(tasks,obs,1000);
  assert.equal(summary.observedCount,4); assert.equal(summary.state,'occupied');
  assert.equal(core.summarizePassengers(tasks.slice(4),obs,1000).state,'unattended');
  assert.equal(core.summarizePassengers([], {...obs,complete:false},1000).state,'unknown');
  assert.equal(core.summarizePassengers([],obs,121001).state,'unknown');
  assert.equal(core.summarizePassengers([{id:'legacy',status:'active'}],obs,1000).state,'unknown');
  assert.equal(core.summarizePassengers([tasks[0],tasks[0]],obs,1000).observedCount,1);
});

test('unattended work uses passenger existence rather than running task count', () => {
  const facts = {workingState:'dirty',integration:'ahead',headRefCoverage:'protected'}, obs = {complete:true,observedAtMs:1000};
  for (const status of ['idle','completed','notLoaded','waiting']) {
    assert.equal(classifyWorkspace(facts,[{id:'chat',status,lifecycle:'present'}],obs,1000).level,'normal');
  }
  assert.ok(classifyWorkspace(facts,[{id:'chat',status:'active',lifecycle:'archived'}],obs,1000).reasons.includes('unattended_work'));
  assert.equal(classifyWorkspace(facts,[],{...obs,complete:false},1000).level,'unknown');
});

test('future journeys end outside history and never create commit edges', () => {
  const core = require(CORE_PATH), layout = layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments);
  const before = JSON.stringify(layout), target = layout.refs[0].ref_name;
  const plan = {route_id:'route-one',worktree_id:layout.attachments[0].worktree_id,target_ref:target,abandoned:false};
  const projected = core.layoutJourneys(layout, [plan]);
  assert.equal(JSON.stringify(layout), before);
  assert.equal(projected.journeys.length, 1);
  assert.ok(projected.arrivals[0].x > layout.width);
  assert.equal(projected.arrivals[0].target_ref, target);
  assert.equal(projected.arrivals[0].available, true);
  assert.equal(core.layoutJourneys(layout, [{...plan,abandoned:true}]).journeys.length, 0);
  assert.equal(core.layoutJourneys(layout, [{...plan,target_ref:'refs/heads/missing'}]).arrivals[0].available, false);
  assert.equal(core.layoutJourneys(layout, [{...plan,worktree_id:'missing'}]).journeys.length, 0);
});
const layoutOptions = { rowGap: 96, columnGap: 96, repositoryId: 'synthetic-repository' };
const point = (node) => ({ x: node.x, y: node.y });
test('compact ref shelf keeps two connected workspace routes within one desktop scene', () => {
  const a = oid('a'), b = oid('b'), c = oid('c');
  const graph = { commits: [{ oid: a, parents: [], subject: null, authored_at: null }, { oid: b, parents: [a], subject: null, authored_at: null }, { oid: c, parents: [a], subject: null, authored_at: null }], refs: [{ ref_name: 'refs/heads/main', display_name: 'main', oid: b, kind: 'branch' }, { ref_name: 'refs/tags/v1', display_name: 'v1', oid: b, kind: 'tag' }, { ref_name: 'refs/heads/feature', display_name: 'feature', oid: c, kind: 'branch' }], edges: [{ id: 'ab', from_oid: a, to_oid: b }, { id: 'ac', from_oid: a, to_oid: c }], boundaries: [], complete: true };
  const layout = layoutTopology(graph, [{worktree_id: 'main', head_oid: b, width:320, height:250}, {worktree_id:'feature', head_oid:c, width:320, height:250}], { rowGap:48, columnGap:96 });
  const stations = [b,c].map(id => layout.nodes.find(n => n.id === id));
  assert.ok(stations[1].y - stations[0].y <= 420, 'ref metadata must not add three separate full-height rows below a workspace');
  assert.ok(layout.attachments.every(a => a.y + a.height < 850), 'both connected workspace groups fit below a compact desktop header');
  assertClearLayout(layout);
});
const segments = (route) => route.points.slice(1).map((end, i) => [route.points[i], end]);
function inSegment(p, a, b) {
  return (p.x - a.x) * (b.y - a.y) === (p.y - a.y) * (b.x - a.x)
    && p.x >= Math.min(a.x, b.x) && p.x <= Math.max(a.x, b.x)
    && p.y >= Math.min(a.y, b.y) && p.y <= Math.max(a.y, b.y);
}
function rectanglesOverlap(a, b) {
  return a.x < b.x + b.width && a.x + a.width > b.x
    && a.y < b.y + b.height && a.y + a.height > b.y;
}
function assertClearLayout(layout) {
  const paths = [...layout.edges, ...layout.attachments.filter(a => a.stem).map(a => a.stem),
    ...layout.refs.map(r => r.stem), ...layout.boundaries.map(b => b.stem)];
  for (const [index, rect] of layout.obstacles.entries()) {
    assert.ok(rect.x >= 0 && rect.y >= 0);
    assert.ok(rect.x + rect.width <= layout.width && rect.y + rect.height <= layout.height);
    for (const other of layout.obstacles.slice(index + 1)) {
      assert.equal(rectanglesOverlap(rect, other), false, rect.id + ' overlaps ' + other.id);
    }
    for (const route of paths) for (const [a, b] of segments(route)) {
      assert.ok(a.x === b.x || a.y === b.y || Math.abs(a.x - b.x) === Math.abs(a.y - b.y));
      // Positive-length penetration of a reserved text/control rectangle is a failure.
      if (a.y === b.y) assert.equal(a.y > rect.y && a.y < rect.y + rect.height
        && Math.max(a.x, b.x) > rect.x && Math.min(a.x, b.x) < rect.x + rect.width, false,
      route.id + ' crosses ' + rect.id);
      if (a.x === b.x) assert.equal(a.x > rect.x && a.x < rect.x + rect.width
        && Math.max(a.y, b.y) > rect.y && Math.min(a.y, b.y) < rect.y + rect.height, false,
      route.id + ' crosses ' + rect.id);
    }
  }
  for (const route of layout.edges) for (const node of layout.nodes) {
    if (node.id === route.from_oid || node.id === route.to_oid) continue;
    assert.equal(segments(route).some(([a, b]) => inSegment(node, a, b)), false,
      'rail crosses unrelated station ' + node.id);
  }
}

test('layout connects every real parent and preserves shared commits and fork-of-feature', () => {
  assert.equal(typeof layoutTopology, 'function', 'layoutTopology must be exported');
  assert.equal(validateTopology(TOPOLOGY.graph).valid, true);
  const layout = layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments, layoutOptions);
  const nodes = new Map(layout.nodes.map(n => [n.id, n]));
  assert.equal(nodes.size, 12);
  assert.equal(layout.edges.length, 12);
  for (const edge of TOPOLOGY.graph.edges) {
    const route = layout.edges.find(e => e.id === edge.id);
    assert.deepEqual(route.points[0], point(nodes.get(edge.from_oid)));
    assert.deepEqual(route.points.at(-1), point(nodes.get(edge.to_oid)));
    assert.ok(nodes.get(edge.from_oid).rank < nodes.get(edge.to_oid).rank);
    assert.ok(nodes.get(edge.from_oid).x < nodes.get(edge.to_oid).x);
  }
  assert.equal(layout.attachments.filter(a => a.head_oid === TOPOLOGY.ids.a2).length, 2);
  assert.equal(layout.nodes.filter(n => n.id === TOPOLOGY.ids.a2).length, 1);
  assert.ok(layout.edges.some(e => e.from_oid === TOPOLOGY.ids.a1 && e.to_oid === TOPOLOGY.ids.u1));
  assert.ok(!layout.edges.some(e => e.from_oid === TOPOLOGY.ids.m1 && e.to_oid === TOPOLOGY.ids.u1));
  assert.ok(layout.refs.some(r => r.ref_name === 'refs/heads/feature/experiment'));
  assert.ok(layout.nodes.some(n => n.id === TOPOLOGY.ids.d1 && n.kind === 'commit'));
  assertClearLayout(layout);
});

test('shuffled inputs and workspace risk changes cannot reorder or recolor history', () => {
  const expected = layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments, layoutOptions);
  const graph = structuredClone(TOPOLOGY.graph);
  for (const key of ['commits', 'refs', 'edges', 'boundaries']) graph[key].reverse();
  assert.deepEqual(layoutTopology(graph, [...TOPOLOGY.attachments].reverse(), layoutOptions), expected);
  assert.deepEqual(layoutTopology(graph, TOPOLOGY.attachments.map(a => ({ ...a, state: 'attention', activeCount: 9 })), layoutOptions), expected);
  const before = JSON.stringify(TOPOLOGY);
  layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments, layoutOptions);
  assert.equal(JSON.stringify(TOPOLOGY), before, 'pure layout must not mutate its inputs');
});

test('older history preserves semantic and lane identities and existing relative order', () => {
  const initial = layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments, layoutOptions);
  const graph = structuredClone(TOPOLOGY.graph);
  const older = 'f'.repeat(40);
  graph.commits.find(c => c.oid === TOPOLOGY.ids.i).parents.push(older);
  graph.commits.push({ oid: older, parents: [], authored_at: null, subject: 'older' });
  graph.edges.push({ id: older + ':' + TOPOLOGY.ids.i, from_oid: older, to_oid: TOPOLOGY.ids.i });
  const result = layoutTopology(graph, TOPOLOGY.attachments, layoutOptions);
  for (const node of initial.nodes) {
    const moved = result.nodes.find(n => n.id === node.id);
    assert.equal(moved.lane_id, node.lane_id);
    assert.equal(moved.rank, node.rank + 1);
  }
  assert.deepEqual(result.lanes.map(l => [l.id, l.color]), initial.lanes.map(l => [l.id, l.color]));
});

test('measured expansion reserves space and reflows fifty workspaces without shrinking', () => {
  const attachments = Array.from({ length: 50 }, (_, i) => ({
    worktree_id: 'workspace-' + String(i).padStart(2, '0'),
    head_oid: Object.values(TOPOLOGY.ids)[i % 12], width: 320, height: 120,
  }));
  const initial = layoutTopology(TOPOLOGY.graph, attachments, layoutOptions);
  const expanded = layoutTopology(TOPOLOGY.graph, attachments.map((a, i) =>
    i === 0 ? { ...a, width: 640, height: 900 } : a), layoutOptions);
  assert.equal(expanded.attachments.length, 50);
  assert.ok(expanded.width > initial.width);
  assert.ok(expanded.height > initial.height);
  assert.deepEqual(expanded.nodes.map(n => [n.id, n.rank, n.lane_id]), initial.nodes.map(n => [n.id, n.rank, n.lane_id]));
  assert.equal(expanded.attachments[0].width, 640);
  assert.equal(expanded.attachments[0].height, 900);
  assertClearLayout(initial);
  assertClearLayout(expanded);
  const defaults = layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments, layoutOptions);
  assert.ok(defaults.attachments.every(a => a.width <= 328 && a.width >= 280));
});

test('boundary annotations coalesce by endpoint and retain every reason and honest action', () => {
  assert.equal(validateTopology(BOUNDARIES.graph).valid, true);
  const layout = layoutTopology(BOUNDARIES.graph, BOUNDARIES.attachments, layoutOptions);
  assert.equal(layout.nodes.length, 3);
  assert.equal(layout.nodes.filter(n => n.kind === 'boundary').length, 2);
  assert.equal(layout.boundaries.length, 4);
  assert.deepEqual(layout.boundaries.map(b => b.id).sort(), ['enrichment', 'limit', 'omitted-ref', 'shallow']);
  assert.ok(layout.boundaries.every(b => b.action === 'explain_boundary'));
  assert.ok(layout.edges.every(e => e.navigation.from.node_id === e.from_oid && e.navigation.to.node_id === e.to_oid));
  assertClearLayout(layout);
});

test('unborn workspaces stay explicitly unanchored with no invented commit or connector', () => {
  const graph = { commits: [], refs: [], edges: [], boundaries: [], complete: true };
  const result = layoutTopology(graph, [
    { worktree_id: 'empty', head_oid: '' }, { worktree_id: 'zero', head_oid: '0'.repeat(40) },
  ], layoutOptions);
  assert.equal(result.nodes.length, 0);
  assert.equal(result.edges.length, 0);
  assert.equal(result.attachments.length, 2);
  assert.ok(result.attachments.every(a => a.kind === 'unborn' && a.stem === null));
  assertClearLayout(result);
});

test('layout rejects unknown ordinary heads and invalid measured dimensions', () => {
  assert.throws(() => layoutTopology(TOPOLOGY.graph, [{ worktree_id: 'missing', head_oid: 'f'.repeat(40) }], layoutOptions), /endpoint/i);
  assert.throws(() => layoutTopology(TOPOLOGY.graph, [...TOPOLOGY.attachments, TOPOLOGY.attachments[0]], layoutOptions), /duplicate/i);
  for (const dimension of [0, -1, Infinity, NaN, 1000000000]) {
    assert.throws(() => layoutTopology(TOPOLOGY.graph, [{ ...TOPOLOGY.attachments[0], height: dimension }], layoutOptions), /dimension/i);
  }
  const invalid = structuredClone(TOPOLOGY.graph);
  invalid.edges.pop();
  assert.throws(() => layoutTopology(invalid, [], layoutOptions), /topology/i);
});

test('every non-node rail crossing provides a visible gap on its actual route', () => {
  const layout = layoutTopology(TOPOLOGY.graph, TOPOLOGY.attachments, layoutOptions);
  let checked = 0;
  for (let i = 0; i < layout.edges.length; i++) for (let j = i + 1; j < layout.edges.length; j++) {
    const one = layout.edges[i], two = layout.edges[j];
    for (const [a, b] of segments(one)) for (const [c, d] of segments(two)) {
      if ((a.y === b.y) === (c.y === d.y)) continue;
      const p = a.y === b.y ? { x: c.x, y: a.y } : { x: a.x, y: c.y };
      if (!inSegment(p, a, b) || !inSegment(p, c, d)) continue;
      if (layout.nodes.some(n => n.x === p.x && n.y === p.y)) continue;
      const crossing = layout.crossings.find(x => x.x === p.x && x.y === p.y
        && [x.over_id, x.under_id].includes(one.id) && [x.over_id, x.under_id].includes(two.id));
      assert.ok(crossing, 'unmarked crossing between ' + one.id + ' and ' + two.id);
      const under = layout.edges.find(e => e.id === crossing.under_id);
      assert.ok(under.gaps.some(g => g.crossing_id === crossing.id));
      assert.ok(crossing.gap[0].x !== crossing.gap[1].x || crossing.gap[0].y !== crossing.gap[1].y);
      assert.ok(crossing.gap.every(p => segments(under).some(([s, e]) => inSegment(p, s, e))));
      checked++;
    }
  }
  assert.ok(checked > 0, 'fixture must exercise real non-node crossings');
});

test('all workspace and annotation stems join the true station to the reserved rectangle', () => {
  for (const fixture of [TOPOLOGY, BOUNDARIES]) {
    const layout = layoutTopology(fixture.graph, fixture.attachments, layoutOptions);
    for (const record of [...layout.attachments, ...layout.refs, ...layout.boundaries]) {
      const node = layout.nodes.find(n => n.id === (record.head_oid || record.oid));
      assert.ok(record.stem);
      assert.equal(record.stem.kind, 'association');
      assert.deepEqual(record.stem.points[0], point(node));
      const end = record.stem.points.at(-1);
      assert.equal(end.x, record.x);
      assert.ok(end.y > record.y && end.y < record.y + record.height, 'stem joins its reserved rectangle');
      if (record.worktree_id) assert.ok(end.y <= record.y + 32, 'workspace association must reach identity before the task roster');
    }
  }
});

test('synthetic fixtures can be consumed by the v4 renderer without relaxing wire validation', () => {
  for (const fixture of [TOPOLOGY, BOUNDARIES]) {
    const snapshot = v4Snapshot(fixture.graph, fixture.attachments[0].head_oid);
    snapshot.workspace_facts = fixture.attachments.map(a => ({
      ...snapshot.workspace_facts[0], worktree_id: a.worktree_id, head_oid: a.head_oid,
    }));
    snapshot.current_worktree_id = fixture.attachments[0].worktree_id;
    snapshot.counts.workspaces = fixture.attachments.length;
    assert.deepEqual(validateSnapshot(snapshot), { valid: true, errors: [] });
  }
});

test('ref-less multi-root octopus and long-parent history routes avoid every unrelated station', () => {
  const commits = Array.from({ length: 30 }, (_, i) => ({
    oid: (i + 1).toString(16).padStart(40, '0'), parents: [], authored_at: null, subject: null,
  }));
  for (let i = 3; i < commits.length; i++) {
    commits[i].parents = [...new Set([i - 1, Math.floor(i / 2), i % 3])].map(p => commits[p].oid);
  }
  const graph = { commits, refs: [], boundaries: [], complete: true,
    edges: commits.flatMap(c => c.parents.map(p => ({ id: p + ':' + c.oid, from_oid: p, to_oid: c.oid }))) };
  const layout = layoutTopology(graph, commits.filter((_, i) => i % 5 === 0).map((c, i) => ({
    worktree_id: 'detached-' + i, head_oid: c.oid,
  })), layoutOptions);
  assert.equal(layout.nodes.length, 30);
  assert.ok(layout.lanes.every(l => l.role === 'neutral' && l.color === null));
  assertClearLayout(layout);
  for (const e of layout.edges) {
    assert.deepEqual(e.points[0], point(layout.nodes.find(n => n.id === e.from_oid)));
    assert.deepEqual(e.points.at(-1), point(layout.nodes.find(n => n.id === e.to_oid)));
  }
});

test('the supported 2048-commit chain lays out without recursive stack growth or truncation', () => {
  const commits = Array.from({ length: 2048 }, (_, i) => ({
    oid: (i + 1).toString(16).padStart(40, '0'),
    parents: i ? [i.toString(16).padStart(40, '0')] : [], authored_at: null, subject: null,
  }));
  const graph = { commits, refs: [], boundaries: [], complete: true,
    edges: commits.slice(1).map(c => ({ id: c.parents[0] + ':' + c.oid, from_oid: c.parents[0], to_oid: c.oid })) };
  const layout = layoutTopology(graph, []);
  assert.equal(layout.nodes.length, 2048);
  assert.equal(layout.edges.length, 2047);
  assert.equal(layout.lanes.length, 1);
  assert.equal(layout.nodes.at(-1).rank, 2047);
  assert.ok(layout.width > 2048 * 48);
  assert.ok(Number.isFinite(layout.width) && Number.isFinite(layout.height));
});

function validGraph() {
  return {
    commits: [
      { oid: oid('a'), parents: [], authored_at: '2026-09-04T10:00:00Z', subject: 'root' },
      { oid: oid('b'), parents: [oid('a')], authored_at: null, subject: null },
    ],
    refs: [
      { ref_name: 'refs/heads/main', display_name: 'main', oid: oid('b'), kind: 'branch' },
      { ref_name: 'refs/tags/v1', display_name: 'v1', oid: oid('b'), kind: 'tag' },
    ],
    edges: [{ id: 'edge-a-b', from_oid: oid('a'), to_oid: oid('b') }],
    boundaries: [],
    complete: true,
  };
}

function v4Snapshot(graph = validGraph(), headOid = oid('b')) {
  return {
    schema_version: 'devmap/dock/4',
    repository_id: `sha256-${oid('c')}${oid('c').slice(0, 24)}`,
    revision: 4,
    observation_revision: 9,
    generated_at: '2026-09-04T10:00:00Z',
    current_worktree_id: `wt-${oid('d')}${oid('d').slice(0, 24)}`,
    development_target: null,
    integration_branches: [],
    branch_groups: [],
    topology: graph,
    workspace_facts: [{
      worktree_id: `wt-${oid('d')}${oid('d').slice(0, 24)}`,
      head_oid: headOid,
      detached: false,
      head_ref_coverage: headOid ? 'protected' : 'unknown',
      integration: headOid ? 'included' : 'terminal',
      target_ref: headOid ? 'refs/heads/main' : null,
      merge_commit_oid: null,
      working_state: 'clean',
      upstream: 'unknown',
      task_observed_at: null,
      git_observed_at: '2026-09-04T10:00:00Z',
      writer_evidence: [],
    }],
    task_observation: { observed_at: null, complete: false },
    counts: { workspaces: 1, tasks: 0 },
    task_inventory_synced_at: null,
    lanes: [],
    current: [],
    active: [],
    stale_or_uninstrumented: [],
    warnings: [],
    truncated: false,
  };
}

function producerChat() {
  return {
    session_id: 'session-one',
    codex_thread_id: '01990000-0000-7000-8000-000000000001',
    display_title: 'Implement metro validation',
    actor_id: 'codex',
    host: 'local',
    host_status: 'active',
    route_id: 'thread:01990000-0000-7000-8000-000000000001',
    status: 'working',
    status_source: 'host_explicit',
    confidence: 'observed',
    capture_grade: 'A',
    last_event_at: '2026-09-04T10:00:00Z',
    blocker_count: 0,
    gap_count: 0,
    capture_incomplete: false,
    association_source: 'codex_task_cwd',
  };
}

function producerRelationship() {
  return {
    base_target: 'main',
    merge_target: 'main',
    merged: false,
    ahead: 1,
    behind: 0,
    dirty: true,
    changed_file_count: 2,
    status_observed: true,
    fork_point: {
      target_branch: 'main',
      commit: oid('a'),
      tags: ['base'],
      subject: 'root',
      authored_at: '2026-09-04T09:00:00Z',
      distance_to_target: 1,
    },
  };
}

function producerLane() {
  return {
    worktree_id: `wt-${oid('d')}${oid('d').slice(0, 24)}`,
    workspace_path: 'C:/workspace/repository',
    is_current: true,
    branch: 'feature/metro',
    head: oid('b'),
    relationship: producerRelationship(),
    chats: [producerChat()],
  };
}

function producerEntry() {
  return {
    worktree_id: `wt-${oid('d')}${oid('d').slice(0, 24)}`,
    display_path: 'C:/workspace/repository',
    is_current: true,
    branch: 'feature/metro',
    head: oid('b'),
    session_id: 'session-one',
    actor_id: 'codex',
    host: 'local',
    route_id: 'thread:01990000-0000-7000-8000-000000000001',
    status: 'working',
    status_source: 'host_explicit',
    confidence: 'observed',
    capture_grade: 'A',
    last_event_at: '2026-09-04T10:00:00Z',
    blocker_count: 0,
    gap_count: 0,
    capture_incomplete: false,
  };
}

function producerShapeSnapshot() {
  const snapshot = v4Snapshot();
  snapshot.development_target = { name: 'main', ref_name: 'refs/heads/main', source: 'local_main' };
  snapshot.integration_branches = [{ name: 'main', ref_name: 'refs/heads/main', head: oid('b'), parent: null, source: 'local_main' }];
  snapshot.branch_groups = [{ target_branch: 'main', terminal: false, fork_point: producerRelationship().fork_point, lanes: [producerLane()] }];
  snapshot.lanes = [producerLane()];
  snapshot.current = [producerEntry()];
  return snapshot;
}

test('attention classification implements every fixed-clock evidence row', async (t) => {
  assert.equal(TASK_FRESHNESS_MS, 120_000);
  assert.equal(IDLE_ATTENTION_MS, 1_800_000);
  for (const scenario of ATTENTION_FIXTURE.cases) {
    await t.test(scenario.name, () => {
      const actual = classifyWorkspace(scenario.facts, scenario.tasks, scenario.observation, ATTENTION_FIXTURE.nowMs);
      assert.deepEqual(actual, scenario.expected);
    });
  }
});

test('idle time never removes an existing passenger', () => {
  const facts = { workingState: 'dirty', integration: 'included', headRefCoverage: 'protected', detached: false, upstream: 'unknown' };
  const tasks = [{ id: 'idle-boundary', lifecycle:'present', status: 'idle', lastActivityMs: 8_200_000, writeObservedAtMs: null }];
  assert.deepEqual(
    classifyWorkspace(facts, tasks, { complete: true, observedAtMs: 9_999_999 }, 9_999_999),
    { level: 'normal', reasons: [], activeCount: 0 },
  );
  assert.deepEqual(
    classifyWorkspace(facts, tasks, { complete: true, observedAtMs: 10_000_000 }, 10_000_000),
    { level: 'normal', reasons: [], activeCount: 0 },
  );
});

test('duplicate task rows do not inflate active or writer counts', () => {
  const facts = { workingState: 'dirty', integration: 'ahead', headRefCoverage: 'protected', detached: false, upstream: 'unknown' };
  const tasks = [
    { id: 'same-task', lifecycle:'present', status: 'working', lastActivityMs: 9_999_000, writeObservedAtMs: 9_999_000 },
    { id: 'same-task', lifecycle:'present', status: 'working', lastActivityMs: 9_998_000, writeObservedAtMs: 9_998_000 },
  ];
  const result = classifyWorkspace(facts, tasks, { complete: true, observedAtMs: 9_999_000 }, 10_000_000);
  assert.equal(result.activeCount, 1);
  assert.ok(!result.reasons.includes('shared_workspace'));
  assert.ok(!result.reasons.includes('concurrent_writes'));
});

test('topology validation accepts real parent edges and multiple refs at one commit', () => {
  assert.deepEqual(validateTopology(validGraph()), { valid: true, errors: [] });
});

test('topology validation accepts a boundary endpoint for an omitted parent', () => {
  const graph = validGraph();
  graph.commits = [{ oid: oid('b'), parents: [oid('a')], authored_at: null, subject: null }];
  graph.boundaries = [{ id: 'boundary-history-a', oid: oid('a'), reason: 'history_limit' }];
  graph.complete = false;
  assert.deepEqual(validateTopology(graph), { valid: true, errors: [] });
});

test('topology validation preserves multiple boundary annotations on a retained commit', () => {
  const graph = validGraph();
  graph.boundaries = [
    { id: 'boundary-shallow-a', oid: oid('a'), reason: 'shallow' },
    { id: 'boundary-unrelated-a', oid: oid('a'), reason: 'unrelated' },
  ];
  assert.deepEqual(validateTopology(graph), { valid: true, errors: [] });
});

test('topology validation rejects duplicates, missing endpoints, invented parents, cycles, enums, and bounds', async (t) => {
  const cases = [
    ['duplicate commit OID', (graph) => graph.commits.push({ ...graph.commits[0] }), 'duplicate_commit_oid'],
    ['duplicate edge ID', (graph) => graph.edges.push({ ...graph.edges[0] }), 'duplicate_edge_id'],
    ['missing endpoint', (graph) => { graph.edges[0].from_oid = oid('e'); graph.commits[1].parents = [oid('e')]; }, 'edge_endpoint_missing'],
    ['invented parent edge', (graph) => { graph.commits[1].parents = []; }, 'edge_not_parent'],
    ['parent without edge', (graph) => { graph.edges = []; }, 'parent_edge_missing'],
    ['cycle', (graph) => {
      graph.commits[0].parents = [oid('b')];
      graph.edges.push({ id: 'edge-b-a', from_oid: oid('b'), to_oid: oid('a') });
    }, 'topology_cycle'],
    ['invalid ref kind', (graph) => { graph.refs[0].kind = 'workspace'; }, 'invalid_ref_kind'],
    ['invalid boundary reason', (graph) => graph.boundaries.push({ id: 'bad-boundary', oid: oid('e'), reason: 'forgotten' }), 'invalid_boundary_reason'],
    ['too many commits', (graph) => {
      graph.commits = Array.from({ length: 2049 }, (_, index) => ({ oid: index.toString(16).padStart(40, '0'), parents: [], authored_at: null, subject: null }));
      graph.refs = [];
      graph.edges = [];
    }, 'commit_count_exceeded'],
  ];
  for (const [name, mutate, error] of cases) {
    await t.test(name, () => {
      const graph = validGraph();
      mutate(graph);
      const result = validateTopology(graph);
      assert.equal(result.valid, false);
      assert.ok(result.errors.includes(error), result.errors.join(', '));
    });
  }
});

test('snapshot validation accepts v4 and documented unborn HEAD sentinels', () => {
  assert.deepEqual(validateSnapshot(v4Snapshot()), { valid: true, errors: [] });
  assert.deepEqual(validateSnapshot(v4Snapshot({ commits: [], refs: [], edges: [], boundaries: [], complete: true }, '')), { valid: true, errors: [] });
  assert.deepEqual(validateSnapshot(v4Snapshot({ commits: [], refs: [], edges: [], boundaries: [], complete: true }, oid('0'))), { valid: true, errors: [] });
});

test('snapshot validation rejects a nonzero worktree HEAD missing from commits and boundaries', () => {
  const result = validateSnapshot(v4Snapshot(validGraph(), oid('e')));
  assert.equal(result.valid, false);
  assert.ok(result.errors.includes('workspace_head_missing'));
});

test('snapshot validation rejects unknown schemas and invalid workspace enums atomically', () => {
  const unknown = v4Snapshot();
  unknown.schema_version = 'devmap/dock/5';
  assert.equal(validateSnapshot(unknown).valid, false);
  assert.ok(validateSnapshot(unknown).errors.includes('unknown_schema_version'));

  const malformed = v4Snapshot();
  malformed.workspace_facts[0].working_state = 'safe';
  const result = validateSnapshot(malformed);
  assert.equal(result.valid, false);
  assert.ok(result.errors.includes('invalid_working_state'));
});

test('snapshot validation accepts the known v3 limited-view envelope', () => {
  const snapshot = v4Snapshot();
  snapshot.schema_version = 'devmap/dock/3';
  delete snapshot.observation_revision;
  delete snapshot.topology;
  delete snapshot.workspace_facts;
  delete snapshot.task_observation;
  delete snapshot.counts;
  assert.deepEqual(validateSnapshot(snapshot), { valid: true, errors: [] });
});

test('snapshot validation accepts a real non-empty v3 relationship without the v4 status observation field', () => {
  const snapshot = producerShapeSnapshot();
  snapshot.schema_version = 'devmap/dock/3';
  delete snapshot.observation_revision;
  delete snapshot.topology;
  delete snapshot.workspace_facts;
  delete snapshot.task_observation;
  delete snapshot.counts;
  delete snapshot.branch_groups[0].lanes[0].relationship.status_observed;
  delete snapshot.lanes[0].relationship.status_observed;
  assert.deepEqual(validateSnapshot(snapshot), { valid: true, errors: [] });
});

test('snapshot validation accepts the complete known producer entry shapes', () => {
  assert.deepEqual(validateSnapshot(producerShapeSnapshot()), { valid: true, errors: [] });
});

test('snapshot validation rejects malformed known producer entries before rendering', async (t) => {
  const cases = [
    ['development target', (snapshot) => { snapshot.development_target.source = 'guessed'; }, 'invalid_development_target'],
    ['top-level Dock entry', (snapshot) => { snapshot.current = [null]; }, 'invalid_dock_entry'],
    ['nested relationship', (snapshot) => { snapshot.branch_groups[0].lanes[0].relationship.changed_file_count = -1; }, 'invalid_relationship'],
    ['nested task', (snapshot) => { snapshot.branch_groups[0].lanes[0].chats[0].status = 'busy'; }, 'invalid_chat'],
  ];
  for (const [name, mutate, error] of cases) {
    await t.test(name, () => {
      const snapshot = producerShapeSnapshot();
      mutate(snapshot);
      const result = validateSnapshot(snapshot);
      assert.equal(result.valid, false);
      assert.ok(result.errors.includes(error), result.errors.join(', '));
    });
  }
});

test('snapshot validation rejects omitted required-nullable producer fields', async (t) => {
  const cases = [
    ['topology metadata', (snapshot) => { delete snapshot.topology.commits[0].authored_at; }, 'topology:invalid_commit_authored_at'],
    ['workspace facts', (snapshot) => { delete snapshot.workspace_facts[0].target_ref; }, 'invalid_target_ref'],
    ['task observation', (snapshot) => { delete snapshot.task_observation.observed_at; }, 'invalid_task_observation'],
    ['integration parent', (snapshot) => { delete snapshot.integration_branches[0].parent; }, 'invalid_integration_branch'],
    ['relationship base', (snapshot) => { delete snapshot.branch_groups[0].lanes[0].relationship.base_target; }, 'invalid_relationship'],
    ['chat host status', (snapshot) => { delete snapshot.branch_groups[0].lanes[0].chats[0].host_status; }, 'invalid_chat'],
    ['Dock entry session', (snapshot) => { delete snapshot.current[0].session_id; }, 'invalid_dock_entry'],
    ['warning subject', (snapshot) => { snapshot.warnings = [{ code: 'example' }]; }, 'invalid_warning'],
    ['inventory timestamp', (snapshot) => { delete snapshot.task_inventory_synced_at; }, 'invalid_task_inventory_time'],
  ];
  for (const [name, mutate, error] of cases) {
    await t.test(name, () => {
      const snapshot = producerShapeSnapshot();
      mutate(snapshot);
      const result = validateSnapshot(snapshot);
      assert.equal(result.valid, false);
      assert.ok(result.errors.includes(error), result.errors.join(', '));
    });
  }
});

test('branch identity is stable, hashes repository plus full ref, and reserves charcoal from branch tokens', () => {
  const first = branchColorKey('sha256-repository-one', 'refs/heads/feature/auth');
  assert.deepEqual(branchColorKey('sha256-repository-one', 'refs/heads/feature/auth'), first);
  assert.notDeepEqual(branchColorKey('sha256-repository-two', 'refs/heads/feature/auth'), first);
  assert.notDeepEqual(branchColorKey('sha256-repository-one', 'refs/heads/feature/api'), first);
  assert.ok(['branch-red', 'branch-blue', 'branch-green', 'branch-yellow', 'branch-cyan', 'branch-magenta'].includes(first.colorToken));
  assert.equal(first.pattern, 'solid', 'dashes are reserved for future intent');
  assert.notEqual(first.colorToken, 'main-charcoal');
});

test('the dependency-free core installs the same API as a browser global', () => {
  const context = { globalThis: {} };
  vm.runInNewContext(fs.readFileSync(CORE_PATH, 'utf8'), context, { filename: 'metro-core.js' });
  assert.equal(typeof context.globalThis.DevMapMetroCore.validateTopology, 'function');
  assert.equal(typeof context.globalThis.DevMapMetroCore.validateSnapshot, 'function');
  assert.equal(typeof context.globalThis.DevMapMetroCore.classifyWorkspace, 'function');
  assert.equal(typeof context.globalThis.DevMapMetroCore.branchColorKey, 'function');
});


test('route plans are bounded intent and cannot impersonate repository facts', () => {
  const value = v4Snapshot();
  const plan = {route_id:'route-0123456789abcdef0123456789abcdef', repository_id:value.repository_id,
    revision:1, worktree_id:value.current_worktree_id, start_commit:oid('a'), goal:'Login',
    target_ref:'refs/heads/main', milestones:['Verify'], source:'User plan', abandoned:false, updated_at:'2026-09-05T00:00:00Z'};
  value.route_plans = [plan];
  assert.equal(validateSnapshot(value).valid, true);
  value.route_plans = [{...plan, milestones:Array(13).fill('Too many')}];
  assert.equal(validateSnapshot(value).valid, false);
  value.route_plans = [{...plan, repository_id:'wrong-repository'}];
  assert.equal(validateSnapshot(value).valid, false);
  value.route_plans = [plan, plan];
  assert.equal(validateSnapshot(value).valid, false);
});
