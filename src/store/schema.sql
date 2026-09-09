CREATE TABLE store_meta (
 singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL,
 repository_id TEXT NOT NULL CHECK(length(repository_id)>0), common_dir TEXT NOT NULL,
 generation INTEGER NOT NULL DEFAULT 0 CHECK(generation>=0),
 backend_state TEXT NOT NULL DEFAULT 'shadow' CHECK(backend_state IN ('shadow','active'))
) STRICT;
CREATE TABLE worktree_registry (
 worktree_id TEXT NOT NULL, incarnation TEXT NOT NULL, git_dir TEXT NOT NULL,
 workspace_path TEXT NOT NULL, retired_at TEXT, PRIMARY KEY(worktree_id,incarnation)
) STRICT;
CREATE TABLE journal_sessions (
 session_id TEXT PRIMARY KEY, worktree_id TEXT NOT NULL, incarnation TEXT NOT NULL,
 origin_path TEXT NOT NULL, FOREIGN KEY(worktree_id,incarnation) REFERENCES worktree_registry(worktree_id,incarnation)
) STRICT;
CREATE TABLE journal_records (
 session_id TEXT NOT NULL REFERENCES journal_sessions(session_id), sequence INTEGER NOT NULL CHECK(sequence>0),
 event_id TEXT NOT NULL CHECK(length(event_id)>0), record_json TEXT NOT NULL CHECK(json_valid(record_json)),
 byte_length INTEGER NOT NULL CHECK(byte_length>=0), PRIMARY KEY(session_id,sequence), UNIQUE(session_id,event_id)
) STRICT;
CREATE TABLE route_records (
 route_id TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), request_id TEXT NOT NULL UNIQUE,
 input_json TEXT NOT NULL CHECK(json_valid(input_json)), plan_json TEXT NOT NULL CHECK(json_valid(plan_json)),
 PRIMARY KEY(route_id,revision)
) STRICT;
CREATE TABLE presence_records (
 session_id TEXT PRIMARY KEY, record_json TEXT NOT NULL CHECK(json_valid(record_json))
) STRICT;
CREATE TABLE binding_records (
 observation_id TEXT PRIMARY KEY, host TEXT NOT NULL, task_id TEXT NOT NULL, observed_at TEXT NOT NULL,
 record_json TEXT NOT NULL CHECK(json_valid(record_json))
) STRICT;
CREATE INDEX binding_task_history ON binding_records(host,task_id,observed_at);
CREATE TABLE binding_watermarks (
 source_scope TEXT PRIMARY KEY, observed_at TEXT NOT NULL, record_json TEXT NOT NULL CHECK(json_valid(record_json))
) STRICT;
CREATE TABLE migration_sources (
 source_path TEXT PRIMARY KEY, source_hash TEXT NOT NULL, record_count INTEGER NOT NULL CHECK(record_count>=0),
 outcome TEXT NOT NULL, record_json TEXT NOT NULL CHECK(json_valid(record_json))
) STRICT;
CREATE TABLE presence_projection (
 session_id TEXT PRIMARY KEY REFERENCES journal_sessions(session_id),
 covered_sequence INTEGER NOT NULL CHECK(covered_sequence>=0),
 covered_sha256 TEXT,
 baseline_source TEXT NOT NULL CHECK(baseline_source IN ('capture','legacy_import'))
) STRICT;
CREATE TABLE journal_heads (
 session_id TEXT PRIMARY KEY REFERENCES journal_sessions(session_id),
 record_count INTEGER NOT NULL CHECK(record_count>=0),
 last_sha256 TEXT,
 byte_length INTEGER NOT NULL CHECK(byte_length>=0),
 CHECK((record_count=0 AND last_sha256 IS NULL) OR (record_count>0 AND length(last_sha256)=64))
) STRICT;
CREATE TABLE route_origin_links (
 route_id TEXT NOT NULL, revision INTEGER NOT NULL,
 worktree_id TEXT NOT NULL CHECK(length(CAST(worktree_id AS BLOB)) BETWEEN 1 AND 256),
 incarnation TEXT CHECK(incarnation IS NULL OR length(CAST(incarnation AS BLOB)) BETWEEN 1 AND 512),
 qualification TEXT NOT NULL CHECK(qualification IN ('native_verified','frozen_baseline','unknown')),
 CHECK((qualification='unknown' AND incarnation IS NULL) OR (qualification!='unknown' AND incarnation IS NOT NULL)),
 PRIMARY KEY(route_id,revision), FOREIGN KEY(route_id,revision) REFERENCES route_records(route_id,revision),
 FOREIGN KEY(worktree_id,incarnation) REFERENCES worktree_registry(worktree_id,incarnation)
) STRICT;
CREATE TABLE binding_origin_links (
 observation_id TEXT PRIMARY KEY REFERENCES binding_records(observation_id),
 destination_worktree_id TEXT NOT NULL CHECK(length(CAST(destination_worktree_id AS BLOB)) BETWEEN 1 AND 256),
 destination_incarnation TEXT CHECK(destination_incarnation IS NULL OR length(CAST(destination_incarnation AS BLOB)) BETWEEN 1 AND 512),
 destination_qualification TEXT NOT NULL CHECK(destination_qualification IN ('native_verified','frozen_baseline','unknown')),
 source_worktree_id TEXT CHECK(source_worktree_id IS NULL OR length(CAST(source_worktree_id AS BLOB)) BETWEEN 1 AND 256),
 source_incarnation TEXT CHECK(source_incarnation IS NULL OR length(CAST(source_incarnation AS BLOB)) BETWEEN 1 AND 512),
 source_qualification TEXT NOT NULL CHECK(source_qualification IN ('native_verified','frozen_baseline','unknown','not_applicable')),
 CHECK((destination_qualification='unknown' AND destination_incarnation IS NULL) OR (destination_qualification!='unknown' AND destination_incarnation IS NOT NULL)),
 CHECK((source_qualification='not_applicable' AND source_worktree_id IS NULL AND source_incarnation IS NULL) OR
       (source_qualification='unknown' AND source_worktree_id IS NOT NULL AND source_incarnation IS NULL) OR
       (source_qualification IN ('native_verified','frozen_baseline') AND source_worktree_id IS NOT NULL AND source_incarnation IS NOT NULL)),
 FOREIGN KEY(destination_worktree_id,destination_incarnation) REFERENCES worktree_registry(worktree_id,incarnation),
 FOREIGN KEY(source_worktree_id,source_incarnation) REFERENCES worktree_registry(worktree_id,incarnation)
) STRICT;
CREATE TABLE binding_origin_cursors (
 source_scope TEXT PRIMARY KEY REFERENCES binding_watermarks(source_scope) CHECK(length(CAST(source_scope AS BLOB)) BETWEEN 1 AND 4096),
 observed_at TEXT NOT NULL CHECK(length(CAST(observed_at AS BLOB)) BETWEEN 1 AND 128),
 current_worktree_id TEXT CHECK(current_worktree_id IS NULL OR length(CAST(current_worktree_id AS BLOB)) BETWEEN 1 AND 256),
 current_incarnation TEXT CHECK(current_incarnation IS NULL OR length(CAST(current_incarnation AS BLOB)) BETWEEN 1 AND 512),
 qualification TEXT NOT NULL CHECK(qualification IN ('native_verified','frozen_baseline','unknown','unobserved')),
 history_observation_id TEXT REFERENCES binding_records(observation_id),
 CHECK((qualification='unobserved' AND current_worktree_id IS NULL AND current_incarnation IS NULL AND history_observation_id IS NULL) OR
       (qualification='unknown' AND current_worktree_id IS NOT NULL AND current_incarnation IS NULL) OR
       (qualification IN ('native_verified','frozen_baseline') AND current_worktree_id IS NOT NULL AND current_incarnation IS NOT NULL)),
 FOREIGN KEY(current_worktree_id,current_incarnation) REFERENCES worktree_registry(worktree_id,incarnation)
) STRICT;
