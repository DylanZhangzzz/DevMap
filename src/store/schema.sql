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
