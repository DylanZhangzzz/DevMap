(function installDevMapMetroCore(root, factory) {
  "use strict";
  const api = factory();
  if (typeof module === "object" && module && module.exports) module.exports = api;
  if (root) root.DevMapMetroCore = api;
})(typeof globalThis === "undefined" ? this : globalThis, function createDevMapMetroCore() {
  "use strict";

  const MAX_COMMITS = 2048;
  const MAX_REFS = 256;
  const MAX_EDGES = 8192;
  const MAX_BOUNDARIES = 4096;
  const MAX_ROWS = 2048;
  const MAX_TEXT = 16384;
  const MAX_SNAPSHOT_COUNT = 1000000;
  const TASK_FRESHNESS_MS = 120000;
  const IDLE_ATTENTION_MS = 1800000;

  const REF_KINDS = new Set(["branch", "remote", "tag"]);
  const BOUNDARY_REASONS = new Set(["history_limit", "shallow", "missing", "unrelated"]);
  const WORKING_STATES = new Set(["clean", "dirty", "unknown"]);
  const INTEGRATION_STATES = new Set(["included", "ahead", "terminal", "unknown"]);
  const HEAD_COVERAGE_STATES = new Set(["protected", "unprotected", "unknown"]);
  const UPSTREAM_STATES = new Set(["published", "local_only", "unknown"]);
  const TARGET_SOURCES = new Set(["config", "local_dev", "local_develop", "remote_default", "local_main", "local_master"]);
  const PRESENCE_STATES = new Set(["starting", "working", "waiting", "idle", "completed", "stale", "unknown"]);
  const STATUS_SOURCES = new Set(["host_explicit", "capture_event", "lease", "git_only"]);
  const CONFIDENCE_STATES = new Set(["observed", "leased", "inferred", "unknown"]);
  const CAPTURE_GRADES = new Set(["A", "B", "C", "D"]);
  const ASSOCIATION_SOURCES = new Set(["presence_worktree_id", "codex_task_cwd"]);
  const ACTIVE_TASK_STATES = new Set(["active", "starting", "working"]);
  const LIFECYCLES = new Set(["present", "archived", "deleted", "unknown"]);
  const TASK_STATES = new Set(["active", "starting", "working", "waiting", "idle", "completed", "stale", "unknown", "notLoaded"]);
  const BRANCH_COLOR_TOKENS = Object.freeze([
    "branch-red",
    "branch-blue",
    "branch-green",
    "branch-yellow",
    "branch-cyan",
    "branch-magenta",
  ]);
  const BRANCH_PATTERNS = Object.freeze(["solid"]);
  const ATTENTION_REASONS = new Set([
    "unattended_work",
    "unprotected_head",
    "concurrent_writes",
  ]);
  const UNKNOWN_REASONS = new Set(["task_activity_unknown", "git_state_unknown"]);

  function isRecord(value) {
    return value !== null && typeof value === "object" && !Array.isArray(value);
  }

  function addError(errors, code) {
    if (!errors.includes(code)) errors.push(code);
  }

  function boundedString(value, allowEmpty) {
    return typeof value === "string" && (allowEmpty || value.length > 0) && value.length <= MAX_TEXT;
  }

  function nullableStringField(value) {
    return value === null || boundedString(value, true);
  }

  function safeOid(value) {
    return typeof value === "string" && (/^[0-9a-f]{40}$/.test(value) || /^[0-9a-f]{64}$/.test(value));
  }

  function unbornHead(value) {
    return value === "" || (safeOid(value) && /^0+$/.test(value));
  }

  function safeInteger(value, maximum) {
    return Number.isSafeInteger(value) && value >= 0 && value <= maximum;
  }

  function boundedArray(value, maximum) {
    return Array.isArray(value) && value.length <= maximum;
  }

  function validRefPrefix(reference) {
    if (reference.kind === "branch") return reference.ref_name.startsWith("refs/heads/");
    if (reference.kind === "remote") return reference.ref_name.startsWith("refs/remotes/");
    if (reference.kind === "tag") return reference.ref_name.startsWith("refs/tags/");
    return false;
  }

  function validateTopology(graph) {
    const errors = [];
    if (!isRecord(graph)) return { valid: false, errors: ["topology_not_object"] };

    if (!boundedArray(graph.commits, MAX_COMMITS)) addError(errors, Array.isArray(graph.commits) ? "commit_count_exceeded" : "commits_not_array");
    if (!boundedArray(graph.refs, MAX_REFS)) addError(errors, Array.isArray(graph.refs) ? "ref_count_exceeded" : "refs_not_array");
    if (!boundedArray(graph.edges, MAX_EDGES)) addError(errors, Array.isArray(graph.edges) ? "edge_count_exceeded" : "edges_not_array");
    if (!boundedArray(graph.boundaries, MAX_BOUNDARIES)) addError(errors, Array.isArray(graph.boundaries) ? "boundary_count_exceeded" : "boundaries_not_array");
    if (typeof graph.complete !== "boolean") addError(errors, "complete_not_boolean");
    if (!Array.isArray(graph.commits) || !Array.isArray(graph.refs) || !Array.isArray(graph.edges) || !Array.isArray(graph.boundaries)) {
      return { valid: false, errors };
    }

    const commits = new Map();
    for (const commit of graph.commits.slice(0, MAX_COMMITS + 1)) {
      if (!isRecord(commit) || !safeOid(commit.oid)) {
        addError(errors, "invalid_commit");
        continue;
      }
      if (commits.has(commit.oid)) addError(errors, "duplicate_commit_oid");
      commits.set(commit.oid, commit);
      if (!boundedArray(commit.parents, MAX_EDGES) || commit.parents.some((parent) => !safeOid(parent))) {
        addError(errors, "invalid_commit_parents");
      } else if (new Set(commit.parents).size !== commit.parents.length) {
        addError(errors, "duplicate_commit_parent");
      }
      if (!nullableStringField(commit.authored_at)) addError(errors, "invalid_commit_authored_at");
      if (!nullableStringField(commit.subject)) addError(errors, "invalid_commit_subject");
    }

    const referenceNames = new Set();
    for (const reference of graph.refs.slice(0, MAX_REFS + 1)) {
      if (!isRecord(reference) || !boundedString(reference.ref_name, false) || !boundedString(reference.display_name, false) || !safeOid(reference.oid)) {
        addError(errors, "invalid_ref");
        continue;
      }
      if (!REF_KINDS.has(reference.kind)) addError(errors, "invalid_ref_kind");
      else if (!validRefPrefix(reference)) addError(errors, "invalid_ref_prefix");
      if (referenceNames.has(reference.ref_name)) addError(errors, "duplicate_ref_name");
      referenceNames.add(reference.ref_name);
    }

    const boundaryIds = new Set();
    const boundaryOids = new Set();
    for (const boundary of graph.boundaries.slice(0, MAX_BOUNDARIES + 1)) {
      if (!isRecord(boundary) || !boundedString(boundary.id, false) || !safeOid(boundary.oid)) {
        addError(errors, "invalid_boundary");
        continue;
      }
      if (!BOUNDARY_REASONS.has(boundary.reason)) addError(errors, "invalid_boundary_reason");
      if (boundaryIds.has(boundary.id)) addError(errors, "duplicate_boundary_id");
      boundaryIds.add(boundary.id);
      boundaryOids.add(boundary.oid);
    }

    const endpoints = new Set([...commits.keys(), ...boundaryOids]);
    for (const reference of graph.refs) {
      if (isRecord(reference) && safeOid(reference.oid) && !endpoints.has(reference.oid)) addError(errors, "ref_endpoint_missing");
    }

    const edgeIds = new Set();
    const edgePairs = new Set();
    for (const edge of graph.edges.slice(0, MAX_EDGES + 1)) {
      if (!isRecord(edge) || !boundedString(edge.id, false) || !safeOid(edge.from_oid) || !safeOid(edge.to_oid)) {
        addError(errors, "invalid_edge");
        continue;
      }
      if (edgeIds.has(edge.id)) addError(errors, "duplicate_edge_id");
      edgeIds.add(edge.id);
      const pair = `${edge.from_oid}\u0000${edge.to_oid}`;
      if (edgePairs.has(pair)) addError(errors, "duplicate_edge");
      edgePairs.add(pair);
      if (!endpoints.has(edge.from_oid) || !commits.has(edge.to_oid)) addError(errors, "edge_endpoint_missing");
      const child = commits.get(edge.to_oid);
      if (child && Array.isArray(child.parents) && !child.parents.includes(edge.from_oid)) addError(errors, "edge_not_parent");
    }

    for (const commit of commits.values()) {
      if (!Array.isArray(commit.parents)) continue;
      for (const parent of commit.parents) {
        if (!edgePairs.has(`${parent}\u0000${commit.oid}`)) addError(errors, "parent_edge_missing");
      }
    }

    const indegree = new Map([...commits.keys()].map((commitOid) => [commitOid, 0]));
    const children = new Map([...commits.keys()].map((commitOid) => [commitOid, []]));
    for (const commit of commits.values()) {
      if (!Array.isArray(commit.parents)) continue;
      for (const parent of commit.parents) {
        if (!commits.has(parent)) continue;
        indegree.set(commit.oid, indegree.get(commit.oid) + 1);
        children.get(parent).push(commit.oid);
      }
    }
    const queue = [];
    for (const [commitOid, degree] of indegree) if (degree === 0) queue.push(commitOid);
    let visited = 0;
    for (let index = 0; index < queue.length; index += 1) {
      const current = queue[index];
      visited += 1;
      for (const child of children.get(current)) {
        const degree = indegree.get(child) - 1;
        indegree.set(child, degree);
        if (degree === 0) queue.push(child);
      }
    }
    if (visited !== commits.size) addError(errors, "topology_cycle");

    return { valid: errors.length === 0, errors };
  }

  function validWorktreeId(value) {
    return typeof value === "string" && /^wt-[0-9a-f]{64}$/.test(value);
  }

  function validRepositoryId(value) {
    return typeof value === "string" && /^sha256-[0-9a-f]{64}$/.test(value);
  }

  function nullableCount(value) {
    return value === null || safeInteger(value, MAX_SNAPSHOT_COUNT);
  }

  function safeRouteId(value) {
    return value === null || (typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9._:/-]{0,511}$/.test(value));
  }

  function safeCodexThreadId(value) {
    return value === null || (typeof value === "string" && /^[A-Za-z0-9][A-Za-z0-9-]{0,255}$/.test(value));
  }

  function validFork(fork) {
    return isRecord(fork)
      && boundedString(fork.target_branch, false)
      && safeOid(fork.commit)
      && boundedArray(fork.tags, 32)
      && fork.tags.every((tag) => boundedString(tag, false))
      && nullableStringField(fork.subject)
      && nullableStringField(fork.authored_at)
      && nullableCount(fork.distance_to_target);
  }

  function validRelationship(relationship, requireStatusObserved) {
    return isRecord(relationship)
      && nullableStringField(relationship.base_target)
      && nullableStringField(relationship.merge_target)
      && (relationship.merged === null || typeof relationship.merged === "boolean")
      && nullableCount(relationship.ahead)
      && nullableCount(relationship.behind)
      && typeof relationship.dirty === "boolean"
      && safeInteger(relationship.changed_file_count, MAX_SNAPSHOT_COUNT)
      && (requireStatusObserved
        ? typeof relationship.status_observed === "boolean"
        : relationship.status_observed === undefined || typeof relationship.status_observed === "boolean")
      && (relationship.fork_point === null || validFork(relationship.fork_point));
  }

  function validChat(chat) {
    return isRecord(chat)
      && (chat.lifecycle === undefined || LIFECYCLES.has(chat.lifecycle))
      && boundedString(chat.session_id, false)
      && safeCodexThreadId(chat.codex_thread_id)
      && boundedString(chat.display_title, false)
      && boundedString(chat.actor_id, false)
      && boundedString(chat.host, false)
      && nullableStringField(chat.host_status)
      && safeRouteId(chat.route_id)
      && PRESENCE_STATES.has(chat.status)
      && STATUS_SOURCES.has(chat.status_source)
      && CONFIDENCE_STATES.has(chat.confidence)
      && CAPTURE_GRADES.has(chat.capture_grade)
      && boundedString(chat.last_event_at, false)
      && safeInteger(chat.blocker_count, MAX_SNAPSHOT_COUNT)
      && safeInteger(chat.gap_count, MAX_SNAPSHOT_COUNT)
      && typeof chat.capture_incomplete === "boolean"
      && ASSOCIATION_SOURCES.has(chat.association_source);
  }

  function validLane(lane, allowUnborn) {
    return isRecord(lane)
      && validWorktreeId(lane.worktree_id)
      && boundedString(lane.workspace_path, false)
      && typeof lane.is_current === "boolean"
      && nullableStringField(lane.branch)
      && (safeOid(lane.head) || (allowUnborn && unbornHead(lane.head)))
      && validRelationship(lane.relationship, allowUnborn)
      && boundedArray(lane.chats, MAX_ROWS)
      && lane.chats.every(validChat);
  }

  function validDockEntry(entry, allowUnborn) {
    return isRecord(entry)
      && validWorktreeId(entry.worktree_id)
      && boundedString(entry.display_path, false)
      && typeof entry.is_current === "boolean"
      && nullableStringField(entry.branch)
      && (safeOid(entry.head) || (allowUnborn && unbornHead(entry.head)))
      && nullableStringField(entry.session_id)
      && nullableStringField(entry.actor_id)
      && nullableStringField(entry.host)
      && safeRouteId(entry.route_id)
      && PRESENCE_STATES.has(entry.status)
      && STATUS_SOURCES.has(entry.status_source)
      && CONFIDENCE_STATES.has(entry.confidence)
      && (entry.capture_grade === null || CAPTURE_GRADES.has(entry.capture_grade))
      && nullableStringField(entry.last_event_at)
      && safeInteger(entry.blocker_count, MAX_SNAPSHOT_COUNT)
      && safeInteger(entry.gap_count, MAX_SNAPSHOT_COUNT)
      && typeof entry.capture_incomplete === "boolean";
  }

  function validateCompatibilityLists(snapshot, errors, allowUnborn) {
    for (const field of ["integration_branches", "branch_groups", "lanes", "current", "active", "stale_or_uninstrumented", "warnings"]) {
      if (!boundedArray(snapshot[field], MAX_ROWS)) addError(errors, Array.isArray(snapshot[field]) ? `${field}_count_exceeded` : `${field}_not_array`);
    }
    if (snapshot.development_target !== null && (!isRecord(snapshot.development_target)
      || !boundedString(snapshot.development_target.name, false)
      || !boundedString(snapshot.development_target.ref_name, false)
      || !TARGET_SOURCES.has(snapshot.development_target.source))) addError(errors, "invalid_development_target");
    if (!Array.isArray(snapshot.integration_branches) || !Array.isArray(snapshot.branch_groups) || !Array.isArray(snapshot.warnings)) return;

    for (const rail of snapshot.integration_branches) {
      if (!isRecord(rail)
        || !boundedString(rail.name, false)
        || !boundedString(rail.ref_name, false)
        || !safeOid(rail.head)
        || !nullableStringField(rail.parent)
        || !TARGET_SOURCES.has(rail.source)) addError(errors, "invalid_integration_branch");
    }
    for (const group of snapshot.branch_groups) {
      if (!isRecord(group)
        || !boundedString(group.target_branch, false)
        || typeof group.terminal !== "boolean"
        || !(group.fork_point === null || validFork(group.fork_point))
        || !boundedArray(group.lanes, MAX_ROWS)) {
        addError(errors, "invalid_branch_group");
        continue;
      }
      for (const lane of group.lanes) {
        if (!isRecord(lane)
          || !validWorktreeId(lane.worktree_id)
          || !boundedString(lane.workspace_path, false)
          || typeof lane.is_current !== "boolean"
          || !nullableStringField(lane.branch)
          || !(safeOid(lane.head) || (allowUnborn && unbornHead(lane.head)))
          || !boundedArray(lane.chats, MAX_ROWS)) {
          addError(errors, "invalid_lane");
          continue;
        }
        if (!validRelationship(lane.relationship, allowUnborn)) addError(errors, "invalid_relationship");
        if (!lane.chats.every(validChat)) addError(errors, "invalid_chat");
      }
    }
    if (Array.isArray(snapshot.lanes)) {
      for (const lane of snapshot.lanes) if (!validLane(lane, allowUnborn)) addError(errors, "invalid_lane");
    }
    for (const field of ["current", "active", "stale_or_uninstrumented"]) {
      if (Array.isArray(snapshot[field])) {
        for (const entry of snapshot[field]) if (!validDockEntry(entry, allowUnborn)) addError(errors, "invalid_dock_entry");
      }
    }
    for (const warning of snapshot.warnings) {
      if (!isRecord(warning) || !boundedString(warning.code, false) || !nullableStringField(warning.subject_id)) addError(errors, "invalid_warning");
    }
  }

  function validateRoutePlans(snapshot, errors) {
    if (snapshot.route_plans === undefined) return;
    if (!Array.isArray(snapshot.route_plans) || snapshot.route_plans.length > 64) { addError(errors, "invalid_route_plans"); return; }
    const ids = new Set();
    const text = (s, max) => typeof s === "string" && s.trim().length > 0 && s.length <= max && !/[\u0000-\u001f\u007f]/.test(s);
    for (const plan of snapshot.route_plans) {
      if (!isRecord(plan) || !/^route-[0-9a-f]{32}$/.test(plan.route_id) || ids.has(plan.route_id)
        || plan.repository_id !== snapshot.repository_id || !validWorktreeId(plan.worktree_id)
        || !safeOid(plan.start_commit) || !safeInteger(plan.revision, Number.MAX_SAFE_INTEGER) || plan.revision < 1
        || !text(plan.goal, 2048) || !text(plan.source, 2048) || !text(plan.updated_at, 128)
        || typeof plan.abandoned !== "boolean"
        || !(plan.target_ref === null || (text(plan.target_ref, 256) && plan.target_ref.startsWith("refs/heads/")))
        || !Array.isArray(plan.milestones) || plan.milestones.length > 12 || plan.milestones.some(item => !text(item, 256))) {
        addError(errors, "invalid_route_plan");
      }
      if (isRecord(plan)) ids.add(plan.route_id);
      if (isRecord(plan) && plan.delivery !== undefined) {
        const d = plan.delivery;
        if (!isRecord(d) || !["manual", "auto_merge"].includes(d.mode)
          || !Array.isArray(d.conditions) || d.conditions.length > 12 || d.conditions.some(c => !text(c, 256))
          || !(d.authorization_source == null || text(d.authorization_source, 2048))
          || (d.mode === "auto_merge" && (!plan.target_ref || !d.conditions.length || !d.authorization_source))) addError(errors, "invalid_delivery");
      }
    }
  }

  function validateSnapshot(snapshot) {
    const errors = [];
    if (!isRecord(snapshot)) return { valid: false, errors: ["snapshot_not_object"] };
    if (snapshot.schema_version !== "devmap/dock/3" && snapshot.schema_version !== "devmap/dock/4") {
      return { valid: false, errors: ["unknown_schema_version"] };
    }
    if (!validRepositoryId(snapshot.repository_id)) addError(errors, "invalid_repository_id");
    if (!validWorktreeId(snapshot.current_worktree_id)) addError(errors, "invalid_current_worktree_id");
    if (!safeInteger(snapshot.revision, Number.MAX_SAFE_INTEGER)) addError(errors, "invalid_revision");
    if (!boundedString(snapshot.generated_at, false)) addError(errors, "invalid_generated_at");
    if (!nullableStringField(snapshot.task_inventory_synced_at)) addError(errors, "invalid_task_inventory_time");
    if (typeof snapshot.truncated !== "boolean") addError(errors, "invalid_truncated");
    validateRoutePlans(snapshot, errors);
    validateCompatibilityLists(snapshot, errors, snapshot.schema_version === "devmap/dock/4");
    if (snapshot.schema_version === "devmap/dock/3") return { valid: errors.length === 0, errors };

    if (!safeInteger(snapshot.observation_revision, Number.MAX_SAFE_INTEGER)) addError(errors, "invalid_observation_revision");
    const topologyResult = validateTopology(snapshot.topology);
    for (const error of topologyResult.errors) addError(errors, `topology:${error}`);

    if (!boundedArray(snapshot.workspace_facts, MAX_ROWS)) {
      addError(errors, Array.isArray(snapshot.workspace_facts) ? "workspace_facts_count_exceeded" : "workspace_facts_not_array");
    }
    if (!isRecord(snapshot.task_observation) || typeof snapshot.task_observation.complete !== "boolean" || !nullableStringField(snapshot.task_observation.observed_at)) {
      addError(errors, "invalid_task_observation");
    } else if (snapshot.task_observation.complete && snapshot.task_observation.observed_at === null) {
      addError(errors, "complete_observation_missing_time");
    }
    if (!isRecord(snapshot.counts) || !safeInteger(snapshot.counts.workspaces, MAX_SNAPSHOT_COUNT) || !safeInteger(snapshot.counts.tasks, MAX_SNAPSHOT_COUNT)) {
      addError(errors, "invalid_counts");
    }

    const commitOids = new Set(Array.isArray(snapshot.topology?.commits) ? snapshot.topology.commits.map((commit) => commit?.oid).filter(safeOid) : []);
    const boundaryOids = new Set(Array.isArray(snapshot.topology?.boundaries) ? snapshot.topology.boundaries.map((boundary) => boundary?.oid).filter(safeOid) : []);
    const workspaceIds = new Set();
    if (Array.isArray(snapshot.workspace_facts)) {
      for (const facts of snapshot.workspace_facts.slice(0, MAX_ROWS + 1)) {
        if (!isRecord(facts) || !validWorktreeId(facts.worktree_id)) {
          addError(errors, "invalid_workspace_facts");
          continue;
        }
        if (workspaceIds.has(facts.worktree_id)) addError(errors, "duplicate_workspace_id");
        workspaceIds.add(facts.worktree_id);
        if (!(safeOid(facts.head_oid) || unbornHead(facts.head_oid))) addError(errors, "invalid_workspace_head");
        else if (!unbornHead(facts.head_oid) && !commitOids.has(facts.head_oid) && !boundaryOids.has(facts.head_oid)) addError(errors, "workspace_head_missing");
        if (typeof facts.detached !== "boolean") addError(errors, "invalid_detached");
        if (!HEAD_COVERAGE_STATES.has(facts.head_ref_coverage)) addError(errors, "invalid_head_ref_coverage");
        if (!INTEGRATION_STATES.has(facts.integration)) addError(errors, "invalid_integration");
        if (!WORKING_STATES.has(facts.working_state)) addError(errors, "invalid_working_state");
        if (!UPSTREAM_STATES.has(facts.upstream)) addError(errors, "invalid_upstream");
        if (!nullableStringField(facts.target_ref)) addError(errors, "invalid_target_ref");
        if (!(facts.merge_commit_oid === null || safeOid(facts.merge_commit_oid))) addError(errors, "invalid_merge_commit_oid");
        if (safeOid(facts.merge_commit_oid) && !commitOids.has(facts.merge_commit_oid) && !boundaryOids.has(facts.merge_commit_oid)) addError(errors, "merge_commit_missing");
        if (!nullableStringField(facts.task_observed_at) || !nullableStringField(facts.git_observed_at)) addError(errors, "invalid_workspace_observation_time");
        if (!boundedArray(facts.writer_evidence, MAX_ROWS)) {
          addError(errors, "invalid_writer_evidence");
        } else {
          const writerIds = new Set();
          for (const evidence of facts.writer_evidence) {
            if (!isRecord(evidence) || !boundedString(evidence.task_id, false) || !boundedString(evidence.observed_at, false) || !boundedString(evidence.source, false)) {
              addError(errors, "invalid_writer_evidence");
              continue;
            }
            if (writerIds.has(evidence.task_id)) addError(errors, "duplicate_writer_task_id");
            writerIds.add(evidence.task_id);
          }
        }
      }
    }
    return { valid: errors.length === 0, errors };
  }

  function finiteInstant(value) {
    return typeof value === "number" && Number.isFinite(value);
  }

  function freshInstant(value, nowMs, thresholdMs) {
    return finiteInstant(value) && finiteInstant(nowMs) && nowMs >= value && nowMs - value <= thresholdMs;
  }

  function classifyWorkspace(facts, tasks, observation, nowMs) {
    const reasons = [];
    const addReason = (reason) => { if (!reasons.includes(reason)) reasons.push(reason); };
    const taskRows = Array.isArray(tasks) ? tasks : [];
    const taskById = new Map();
    for (const task of taskRows) {
      if (!isRecord(task) || !boundedString(task.id, false) || !TASK_STATES.has(task.status) || task.lifecycle !== "present") continue;
      let aggregate = taskById.get(task.id);
      if (!aggregate) {
        aggregate = { active: false, idle: false, lastActivityMs: null, writeObservedAtMs: null };
        taskById.set(task.id, aggregate);
      }
      aggregate.active ||= ACTIVE_TASK_STATES.has(task.status);
      aggregate.idle ||= task.status === "idle";
      if (finiteInstant(task.lastActivityMs) && (aggregate.lastActivityMs === null || task.lastActivityMs > aggregate.lastActivityMs)) aggregate.lastActivityMs = task.lastActivityMs;
      if (finiteInstant(task.writeObservedAtMs) && (aggregate.writeObservedAtMs === null || task.writeObservedAtMs > aggregate.writeObservedAtMs)) aggregate.writeObservedAtMs = task.writeObservedAtMs;
    }

    const uniqueTasks = [...taskById.values()];
    const activeTasks = uniqueTasks.filter((task) => task.active);
    const activeCount = activeTasks.length;
    const observationFresh = isRecord(observation)
      && observation.complete === true
      && freshInstant(observation.observedAtMs, nowMs, TASK_FRESHNESS_MS);

    const workingState = isRecord(facts) ? facts.workingState : "unknown";
    const integration = isRecord(facts) ? facts.integration : "unknown";
    if (!WORKING_STATES.has(workingState) || workingState === "unknown") addReason("git_state_unknown");

    const passengers = summarizePassengers(taskRows, observation, nowMs);
    if (workingState === "dirty" || integration === "ahead") {
      if (passengers.state === "unknown") addReason("task_activity_unknown");
      else if (passengers.state === "unattended") addReason("unattended_work");
    }

    if (isRecord(facts) && facts.detached === true && facts.headRefCoverage === "unprotected") addReason("unprotected_head");

    if (passengers.state === "occupied" && passengers.observedCount >= 2) {
      addReason("shared_workspace");
      const freshWriterCount = activeTasks.filter((task) => freshInstant(task.writeObservedAtMs, nowMs, TASK_FRESHNESS_MS)).length;
      if (freshWriterCount >= 2) addReason("concurrent_writes");
    }

    const level = reasons.some((reason) => ATTENTION_REASONS.has(reason))
      ? "attention"
      : reasons.some((reason) => UNKNOWN_REASONS.has(reason)) ? "unknown" : "normal";
    return { level, reasons, activeCount };
  }

  function summarizePassengers(tasks, observation, nowMs) {
    const chats = new Map();
    for (const task of Array.isArray(tasks) ? tasks : []) {
      if (!isRecord(task) || task.isChat === false || !boundedString(task.id, false)) continue;
      const lifecycle = LIFECYCLES.has(task.lifecycle) ? task.lifecycle : "unknown";
      const previous = chats.get(task.id);
      chats.set(task.id, previous && previous.lifecycle !== lifecycle ? {lifecycle:"unknown",status:"unknown"} : {...task,lifecycle});
    }
    const present = [...chats.values()].filter(t => t.lifecycle === "present");
    const fresh = observation?.complete === true && freshInstant(observation.observedAtMs, nowMs, TASK_FRESHNESS_MS);
    const state = !fresh ? "unknown" : present.length ? "occupied" : [...chats.values()].some(t => t.lifecycle === "unknown") ? "unknown" : "unattended";
    const developing = present.filter(t => ACTIVE_TASK_STATES.has(t.status)).length;
    const waiting = present.filter(t => ["waiting","idle"].includes(t.status)).length;
    const completed = present.filter(t => t.status === "completed").length;
    return {observedCount:present.length,state,developing,waiting,completed,unknown:present.length-developing-waiting-completed};
  }

  function branchColorKey(repositoryId, refName) {
    if (!boundedString(repositoryId, false) || !boundedString(refName, false)) throw new TypeError("repositoryId and refName must be non-empty bounded strings");
    const identity = `${repositoryId}\u0000${refName}`;
    let hash = 2166136261;
    for (let index = 0; index < identity.length; index += 1) {
      hash ^= identity.charCodeAt(index);
      hash = Math.imul(hash, 16777619) >>> 0;
    }
    return {
      identityKey: identity,
      colorToken: BRANCH_COLOR_TOKENS[hash % BRANCH_COLOR_TOKENS.length],
      pattern: BRANCH_PATTERNS[Math.floor(hash / BRANCH_COLOR_TOKENS.length) % BRANCH_PATTERNS.length],
    };
  }

  // Coordinates are CSS-pixel world units. Lanes are deterministic visual chains,
  // never a claim that Git stores permanent branch ownership or branch parents.
  function layoutTopology(graph, attachments = [], options = {}) {
    const validation = validateTopology(graph);
    if (!validation.valid) throw new TypeError(`Invalid topology: ${validation.errors.join(", ")}`);
    if (!boundedArray(attachments, MAX_ROWS)) throw new TypeError("Invalid attachments");
    if (!isRecord(options)) throw new TypeError("Invalid layout options");
    const dimension = (value, fallback) => {
      if (value === undefined) return fallback;
      if (!Number.isFinite(value) || value < 1 || value > 16384) throw new TypeError("Invalid layout dimension (1..16384)");
      return value;
    };
    const rowGap = Math.max(48, dimension(options.rowGap, 96));
    const columnGap = Math.max(48, dimension(options.columnGap, 96));
    const repositoryId = options.repositoryId === undefined ? "repository" : options.repositoryId;
    if (!boundedString(repositoryId, false)) throw new TypeError("Invalid repositoryId");
    const compare = (a, b) => a < b ? -1 : a > b ? 1 : 0;
    const byId = (a, b) => compare(a.id, b.id);
    const at = (n) => ({ x: n.x, y: n.y });
    const commits = new Map(graph.commits.map(c => [c.oid, c]));
    const annotations = [...graph.boundaries].sort(byId);
    const endpoints = new Set([...commits.keys(), ...annotations.map(b => b.oid)]);
    const inputs = [...attachments].map(a => {
      if (!isRecord(a) || !boundedString(a.worktree_id, false)) throw new TypeError("Invalid workspace attachment");
      if (!unbornHead(a.head_oid) && !endpoints.has(a.head_oid)) throw new TypeError("Workspace endpoint missing");
      return { worktree_id: a.worktree_id, head_oid: a.head_oid,
        width: dimension(a.width, 320), height: dimension(a.height, 128) };
    }).sort((a, b) => compare(a.worktree_id, b.worktree_id));
    if (new Set(inputs.map(a => a.worktree_id)).size !== inputs.length) throw new TypeError("Duplicate workspace attachment");
    const edgesInput = [...graph.edges].sort(byId);
    const children = new Map([...endpoints].map(id => [id, []]));
    const indegree = new Map([...endpoints].map(id => [id, 0]));
    const ranks = new Map([...endpoints].map(id => [id, 0]));
    for (const e of edgesInput) {
      children.get(e.from_oid).push(e.to_oid);
      indegree.set(e.to_oid, indegree.get(e.to_oid) + 1);
    }
    const pending = [...endpoints].filter(id => indegree.get(id) === 0).sort(compare);
    const order = [];
    while (pending.length) {
      const id = pending.shift();
      order.push(id);
      for (const child of children.get(id)) {
        ranks.set(child, Math.max(ranks.get(child), ranks.get(id) + 1));
        indegree.set(child, indegree.get(child) - 1);
        if (indegree.get(child) === 0) { pending.push(child); pending.sort(compare); }
      }
    }
    const laneFor = new Map();
    const lanes = [];
    const refPriority = r => r.ref_name === "refs/heads/main" ? 0
      : r.ref_name === "refs/heads/master" ? 1 : r.kind === "branch" ? 2 : r.kind === "remote" ? 3 : 4;
    const refsInput = [...graph.refs].sort((a, b) => refPriority(a) - refPriority(b) || compare(a.ref_name, b.ref_name));
    function assignChain(tip, reference) {
      if (laneFor.has(tip)) return;
      const id = reference ? `ref:${reference.ref_name}` : `history:${tip}`;
      const lane = { id, ref_name: reference ? reference.ref_name : null,
        role: reference && refPriority(reference) < 2 ? "main" : reference ? "branch" : "neutral",
        color: reference ? branchColorKey(repositoryId, reference.ref_name) : null,
        node_ids: [], index: lanes.length };
      lanes.push(lane);
      let next = tip;
      while (next && !laneFor.has(next)) {
        laneFor.set(next, lane);
        lane.node_ids.push(next);
        const c = commits.get(next);
        next = c && c.parents[0];
      }
      lane.node_ids.reverse();
    }
    for (const reference of refsInput) assignChain(reference.oid, reference);
    // Ref-less tips cover detached and branch-deleted history without inventing refs.
    for (const id of [...order].reverse()) assignChain(id, null);
    const nodes = order.map(id => ({ id, rank: ranks.get(id), lane_id: laneFor.get(id).id,
      kind: commits.has(id) ? "commit" : "boundary",
      boundary_ids: annotations.filter(b => b.oid === id).map(b => b.id) }));
    const nodeMap = new Map(nodes.map(n => [n.id, n]));
    const cells = new Map(nodes.map(n => [n.id, []]));
    const label = (kind, id, oid, width, height, payload) => {
      const record = { kind, id, oid, width, height, ...payload };
      cells.get(oid).push(record);
      return record;
    };
    for (const n of nodes) {
      n.transfer = commits.get(n.id)?.parents.length > 1 ? "merge" : children.get(n.id).length > 1 ? "fork" : null;
      label("node-label", `node:${n.id}`, n.id, n.transfer ? 160 : 96, 44, { node: n });
    }
    const refs = [...refsInput].sort((a, b) => compare(a.ref_name, b.ref_name)).map(r =>
      label("ref", `ref:${r.ref_name}`, r.oid, Math.min(320, Math.max(96, r.display_name.length * 8 + 48)), 44, { ref_name: r.ref_name,
        display_name: r.display_name, ref_kind: r.kind, color: branchColorKey(repositoryId, r.ref_name) }));
    const boundaries = annotations.map(b => label("boundary-label", b.id, b.oid, 320, 44,
      { reason: b.reason, action: "explain_boundary" }));
    const anchored = inputs.filter(a => !unbornHead(a.head_oid)).map(a =>
      label("workspace", `workspace:${a.worktree_id}`, a.head_oid, a.width, a.height,
        { worktree_id: a.worktree_id, head_oid: a.head_oid }));
    // Put checkout identity and active previews next to their station. Ref/OID
    // metadata remains in the same collision-free cell below all workspaces.
    const preferredWorkspaces = new Set(options.preferredWorkspaces || []);
    for (const records of cells.values()) records.sort((a, b) => Number(b.kind === "workspace") - Number(a.kind === "workspace")
      || Number(preferredWorkspaces.has(b.worktree_id)) - Number(preferredWorkspaces.has(a.worktree_id)));
    // Workspace previews occupy their own measured rows. Compact metadata shares
    // a shelf; fixed control rectangles still bound text and all connection paths.
    const cellHeights = new Map();
    for (const [id, records] of cells) {
      const width = Math.max(320, ...records.map(record => record.width));
      let top = 24, left = 0, shelfHeight = 0;
      for (const record of records) {
        if (record.kind === "workspace") {
          record.offsetX = 0; record.offsetY = top; top += record.height + 8;
        } else {
          if (left && left + record.width > width) { top += shelfHeight + 8; left = 0; shelfHeight = 0; }
          record.offsetX = left; record.offsetY = top;
          left += record.width + 8; shelfHeight = Math.max(shelfHeight, record.height);
        }
      }
      cellHeights.set(id, top + shelfHeight);
    }
    const rankCount = Math.max(0, ...nodes.map(n => n.rank + 1));
    const columnWidths = Array(rankCount).fill(48);
    const rowHeights = new Map(lanes.map(l => [l.id, 0]));
    for (const n of nodes) {
      const records = cells.get(n.id);
      columnWidths[n.rank] = Math.max(columnWidths[n.rank], ...records.map(r => r.offsetX + r.width + 24));
      rowHeights.set(n.lane_id, Math.max(rowHeights.get(n.lane_id), cellHeights.get(n.id)));
    }
    const trackCounts = new Map(lanes.map(l => [l.id, 0]));
    const gaps = Array.from({ length: rankCount }, () => []);
    const routes = edgesInput.map(e => {
      const from = nodeMap.get(e.from_oid), to = nodeMap.get(e.to_oid);
      const direct = from.lane_id === to.lane_id && !nodes.some(n => n.lane_id === from.lane_id && n.rank > from.rank && n.rank < to.rank);
      const route = { ...e, direct };
      if (!direct) {
        route.track = trackCounts.get(from.lane_id);
        trackCounts.set(from.lane_id, route.track + 1);
        gaps[from.rank].push({ route, key: "departure" });
        gaps[to.rank - 1].push({ route, key: "arrival" });
      }
      return route;
    });
    const columnXs = [];
    let x = 24;
    for (let rank = 0; rank < rankCount; rank++) {
      columnXs.push(x);
      // Each vertical channel is unique; departure precedes arrival in a gap.
      gaps[rank].sort((a, b) => compare(b.key, a.key) || byId(a.route, b.route));
      gaps[rank].forEach((entry, i) => { entry.route[entry.key] = x + columnWidths[rank] + 24 + i * 16; });
      x += columnWidths[rank] + columnGap + gaps[rank].length * 16;
    }
    let y = 24;
    for (const lane of lanes) {
      lane.top = y;
      lane.y = y + trackCounts.get(lane.id) * 16 + 24;
      lane.height = lane.y - lane.top + rowHeights.get(lane.id) + rowGap;
      y += lane.height;
    }
    const obstacles = [];
    for (const n of nodes) {
      n.x = columnXs[n.rank];
      n.y = laneFor.get(n.id).y;
      n.radius = 10;
      for (const record of cells.get(n.id)) {
        const top = n.y + record.offsetY;
        record.x = n.x + 24 + record.offsetX;
        record.y = top;
        record.lane_id = n.lane_id;
        const rect = { id: `${record.kind}:${record.id}`, kind: record.kind, x: record.x, y: top, width: record.width, height: record.height };
        obstacles.push(rect);
        if (record.kind === "node-label") n.label_rect = rect;
        else {
          const stemY = top + (record.kind === "workspace" ? 22 : record.height / 2);
          record.stem = { id: `stem:${record.kind}:${record.id}`, kind: "association", points: record.offsetX
            ? [at(n), { x: n.x, y: top - 4 }, { x: record.x - 4, y: top - 4 }, { x: record.x - 4, y: stemY }, { x: record.x, y: stemY }]
            : [at(n), { x: n.x, y: stemY }, { x: record.x, y: stemY }] };
        }
      }
    }
    const navigate = id => {
      const n = nodeMap.get(id);
      return { node_id: id, action: n.kind === "boundary" ? "explain_boundary" : "locate_node", boundary_ids: n.boundary_ids };
    };
    const edges = routes.map(route => {
      const from = nodeMap.get(route.from_oid), to = nodeMap.get(route.to_oid);
      const trackY = from.y - 16 * (route.track + 1);
      const points = route.direct ? [at(from), at(to)] : [at(from),
        { x: route.departure, y: from.y }, { x: route.departure, y: trackY },
        { x: route.arrival, y: trackY }, { x: route.arrival, y: to.y }, at(to)];
      return { id: route.id, from_oid: route.from_oid, to_oid: route.to_oid,
        kind: "parent", lane_id: from.lane_id === to.lane_id ? to.lane_id
          : commits.get(to.id).parents[0] === from.id ? to.lane_id : from.lane_id,
        points, gaps: [], navigation: { from: navigate(from.id), to: navigate(to.id) } };
    });
    const crossings = railCrossings(edges, nodes);
    const unanchored = inputs.filter(a => unbornHead(a.head_oid)).map(a => {
      const record = { ...a, kind: "unborn", id: `workspace:${a.worktree_id}`, x: 24, y, lane_id: null, stem: null };
      obstacles.push({ id: `workspace:${record.id}`, kind: "workspace", x: record.x, y, width: record.width, height: record.height });
      y += a.height + rowGap;
      return record;
    });
    const workspaceOutput = [...anchored, ...unanchored].sort((a, b) => compare(a.worktree_id, b.worktree_id));
    const width = Math.max(48, x, ...obstacles.map(r => r.x + r.width + 24));
    const height = Math.max(48, y, ...obstacles.map(r => r.y + r.height + 24));
    return { nodes, edges, attachments: workspaceOutput, lanes, refs, boundaries, obstacles, crossings, width, height };
  }

  // Sweep vertical channels across horizontal rails. Gaps erase only the named
  // under-route (SVG mask or split path), never a surface-colored global overlay.
  function railCrossings(edges, nodes) {
    const events = [];
    for (const route of edges) route.points.slice(1).forEach((b, index) => {
      const a = route.points[index];
      if (a.x === b.x && a.y === b.y) return;
      const segment = { route, index, a, b };
      if (a.y === b.y) {
        events.push({ x: Math.min(a.x, b.x), kind: 0, segment });
        events.push({ x: Math.max(a.x, b.x), kind: 2, segment });
      } else events.push({ x: a.x, kind: 1, segment });
    });
    events.sort((a, b) => a.x - b.x || a.kind - b.kind);
    const stations = new Set(nodes.map(n => `${n.x},${n.y}`));
    const active = new Set(), seen = new Set(), crossings = [];
    for (const event of events) {
      const vertical = event.segment;
      if (event.kind === 0) { active.add(vertical); continue; }
      if (event.kind === 2) { active.delete(vertical); continue; }
      for (const horizontal of active) {
        const y = horizontal.a.y;
        if (horizontal.route === vertical.route || y < Math.min(vertical.a.y, vertical.b.y)
          || y > Math.max(vertical.a.y, vertical.b.y) || stations.has(`${event.x},${y}`)) continue;
        const id = JSON.stringify([horizontal.route.id, vertical.route.id, event.x, y]);
        if (seen.has(id)) continue;
        seen.add(id);
        const gap = [{ x: Math.max(Math.min(horizontal.a.x, horizontal.b.x), event.x - 6), y },
          { x: Math.min(Math.max(horizontal.a.x, horizontal.b.x), event.x + 6), y }];
        crossings.push({ id, x: event.x, y, over_id: vertical.route.id, under_id: horizontal.route.id, gap });
        horizontal.route.gaps.push({ crossing_id: id, segment_index: horizontal.index, points: gap });
      }
    }
    return crossings;
  }

  function layoutJourneys(layout, plans = []) {
    if (!Array.isArray(plans) || plans.length > 64) throw new TypeError("Invalid route plans");
    const active = plans.filter(p => !p.abandoned && layout.attachments.some(a => a.worktree_id === p.worktree_id));
    const arrivals = [], journeys = [], groups = new Map();
    const zoneX = layout.width + 80 + active.length * 12;
    for (const [index, plan] of active.entries()) {
      const key = plan.target_ref || "unknown";
      let zone = groups.get(key);
      if (!zone) {
        const ref = layout.refs.find(r => r.ref_name === plan.target_ref);
        const node = ref && layout.nodes.find(n => n.id === ref.oid);
        let y = node ? node.y : layout.height + 40;
        while (arrivals.some(a => Math.abs(a.y - y) < 80)) y += 80;
        zone = {target_ref:plan.target_ref,available:!!node,x:zoneX,y,width:240,height:64,route_ids:[]};
        groups.set(key, zone); arrivals.push(zone);
      }
      zone.route_ids.push(plan.route_id);
      const a = layout.attachments.find(a => a.worktree_id === plan.worktree_id);
      const lane = layout.lanes.find(l => l.id === a.lane_id);
      const shelf = [...layout.refs, ...layout.boundaries, ...layout.attachments].filter(r => r.oid === a.oid);
      const exitX = Math.max(a.x + a.width, ...shelf.map(r => r.x + r.width)) + 12;
      const corridorY = lane ? lane.top + lane.height - 12 : a.y + a.height + 16;
      const channelX = layout.width + 32 + index * 12;
      journeys.push({route_id:plan.route_id,worktree_id:plan.worktree_id,target_ref:plan.target_ref,
        points:[{x:a.x+a.width,y:a.y+a.height-12},{x:exitX,y:a.y+a.height-12},
          {x:exitX,y:corridorY},{x:channelX,y:corridorY},{x:channelX,y:zone.y},{x:zone.x,y:zone.y}]});
    }
    return {journeys,arrivals,width:active.length ? zoneX+264 : layout.width,
      height:Math.max(layout.height,...arrivals.map(a => a.y+a.height))};
  }

  return Object.freeze({
    MAX_COMMITS,
    MAX_REFS,
    MAX_EDGES,
    MAX_BOUNDARIES,
    TASK_FRESHNESS_MS,
    IDLE_ATTENTION_MS,
    validateTopology,
    validateSnapshot,
    classifyWorkspace,
    summarizePassengers,
    branchColorKey,
    layoutTopology,
    layoutJourneys,
  });
});
