mod support;

use devmap::{
    git::{SourceGitInspector, SourceWorkspace},
    store::RepositoryStore,
    worktrees::repository_id,
};
use rusqlite::{Connection, ErrorCode, params};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

// Exact schema from d2dd63f. This is a genuine schema-1 database fixture,
// independent of whatever schema the candidate now creates.
const SCHEMA_ONE: &str = r#"
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
"#;
const IDENTITY_TABLES: [&str; 3] = [
    "route_origin_links",
    "binding_origin_links",
    "binding_origin_cursors",
];
fn source() -> (tempfile::TempDir, SourceWorkspace) {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    (repo, workspace)
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, path: &Path, output: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                output.insert(relative, None);
                walk(root, &path, output);
            } else {
                output.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut output = BTreeMap::new();
    walk(root, root, &mut output);
    output
}
#[test]
fn fresh_store_is_schema_two_with_three_strict_identity_tables() {
    let (_repo, workspace) = source();
    let store = RepositoryStore::open(&workspace).unwrap();
    let database = Connection::open(store.path()).unwrap();
    let version: i64 = database
        .query_row("SELECT schema_version FROM store_meta", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        version, 2,
        "native origin links require an explicit schema version"
    );
    for table in IDENTITY_TABLES {
        let strict: i64 = database
            .query_row(
                "SELECT strict FROM pragma_table_list WHERE name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(strict, 1, "{table} must be STRICT");
    }
}
#[test]
fn missing_mandatory_identity_table_is_rejected_without_repair() {
    for table in IDENTITY_TABLES {
        let (_repo, workspace) = source();
        let store = RepositoryStore::open(&workspace).unwrap();
        let path = store.path().to_path_buf();
        drop(store);
        let database = Connection::open(&path).unwrap();
        database
            .execute_batch(&format!("DROP TABLE IF EXISTS {table}"))
            .unwrap();
        drop(database);
        assert!(
            RepositoryStore::open_existing(&workspace).is_err(),
            "missing {table} was accepted"
        );
        let database = Connection::open(&path).unwrap();
        let count: i64 = database
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name=?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "read repaired a mandatory table");
    }
}
#[test]
fn genuine_schema_one_without_transition_is_rejected_without_side_effects() {
    for writable_open in [false, true] {
        let (_repo, workspace) = source();
        let root = workspace.git_common_dir.join("devmap");
        fs::create_dir(&root).unwrap();
        let database = Connection::open(root.join("devmap.db")).unwrap();
        database.execute_batch(SCHEMA_ONE).unwrap();
        database.execute("INSERT INTO store_meta(singleton,schema_version,repository_id,common_dir) VALUES(1,1,?1,?2)", params![repository_id(&workspace),fs::canonicalize(&workspace.git_common_dir).unwrap().to_string_lossy()]).unwrap();
        assert_eq!(
            database
                .query_row(
                    "SELECT count(*) FROM sqlite_schema WHERE type='table'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            11
        );
        drop(database);
        assert!(!root.join("backend-transition").exists());
        let before = snapshot(&root);
        let rejected = if writable_open {
            RepositoryStore::open(&workspace).map(|_| ())
        } else {
            RepositoryStore::open_existing(&workspace).map(|_| ())
        };
        let error = rejected
            .expect_err("schema 1 was accepted by the schema 2 candidate")
            .to_string();
        assert!(error.contains("version"), "wrong rejection: {error}");
        assert_eq!(
            snapshot(&root),
            before,
            "unsupported version created or changed storage artifacts"
        );
    }
}
#[test]
fn genuine_schema_one_with_existing_gate_does_not_create_initialization_lock() {
    let (_repo, workspace) = source();
    // Let the platform create a valid private transition gate. Replace only
    // this disposable fixture's closed database with the genuine old schema.
    let store = RepositoryStore::open(&workspace).unwrap();
    let path = store.path().to_path_buf();
    drop(store);
    let root = path.parent().unwrap();
    assert!(root.join("backend-transition/lock").is_file());
    fs::remove_file(&path).unwrap();
    fs::remove_file(root.join("store-init.lock")).unwrap();
    let database = Connection::open(&path).unwrap();
    database.execute_batch(SCHEMA_ONE).unwrap();
    database.execute("INSERT INTO store_meta(singleton,schema_version,repository_id,common_dir) VALUES(1,1,?1,?2)", params![repository_id(&workspace),fs::canonicalize(&workspace.git_common_dir).unwrap().to_string_lossy()]).unwrap();
    drop(database);
    let before = snapshot(root);
    let error = RepositoryStore::open(&workspace)
        .err()
        .expect("schema 1 was accepted with an existing transition gate")
        .to_string();
    assert!(error.contains("version"), "wrong rejection: {error}");
    assert!(
        !root.join("store-init.lock").exists(),
        "unsupported version created an initialization lock"
    );
    assert_eq!(snapshot(root), before);
}
fn ddl() -> Connection {
    let database = Connection::open_in_memory().unwrap();
    database.pragma_update(None, "foreign_keys", true).unwrap();
    database
        .execute_batch(include_str!("../src/store/schema.sql"))
        .unwrap();
    database.execute("INSERT INTO worktree_registry(worktree_id,incarnation,git_dir,workspace_path) VALUES('wt','inc','git','root')", []).unwrap();
    database
}
fn constraint(result: rusqlite::Result<usize>) {
    let error = result.expect_err("invalid identity combination was accepted");
    assert_eq!(
        error.sqlite_error_code(),
        Some(ErrorCode::ConstraintViolation),
        "expected a real constraint rejection, not a missing table/column: {error}"
    );
}
#[test]
fn route_identity_qualification_null_and_registry_foreign_keys_are_enforced() {
    let database = ddl();
    // Domain JSON is not projected here: this case isolates relational checks.
    database
        .execute(
            "INSERT INTO route_records VALUES('route',1,'request','{}','{}')",
            [],
        )
        .unwrap();
    let insert = |incarnation: Option<&str>, qualification: &str| {
        database.execute(
        "INSERT INTO route_origin_links(route_id,revision,worktree_id,incarnation,qualification) VALUES('route',1,'wt',?1,?2)", params![incarnation,qualification])
    };
    constraint(insert(None, "native_verified"));
    constraint(insert(Some("inc"), "unknown"));
    constraint(insert(Some("inc"), "invented"));
    constraint(insert(Some("missing"), "native_verified"));
    insert(Some("inc"), "native_verified").unwrap();
    constraint(database.execute("INSERT INTO route_origin_links(route_id,revision,worktree_id,incarnation,qualification) VALUES('absent',1,'wt','inc','native_verified')", []));
}
#[test]
fn binding_absent_source_and_unobserved_cursor_are_distinct_from_unknown_identity() {
    let database = ddl();
    database.execute("INSERT INTO binding_records VALUES('observation','host','task','2026-09-09T00:00:00Z','{}')", []).unwrap();
    let insert = |source: Option<&str>, incarnation: Option<&str>, qualification: &str| {
        database.execute(
        "INSERT INTO binding_origin_links(observation_id,destination_worktree_id,destination_incarnation,destination_qualification,source_worktree_id,source_incarnation,source_qualification) VALUES('observation','wt','inc','native_verified',?1,?2,?3)", params![source,incarnation,qualification])
    };
    constraint(insert(None, None, "unknown"));
    constraint(insert(Some("wt"), None, "not_applicable"));
    constraint(insert(Some("wt"), Some("missing"), "native_verified"));
    insert(None, None, "not_applicable").unwrap();
    let scope = "[\"host\",\"task\"]";
    database.execute("INSERT INTO binding_watermarks VALUES(?1,'2026-09-09T00:00:00Z','[\"host\",\"task\",\"2026-09-09T00:00:00Z\"]')", [scope]).unwrap();
    let cursor = |current: Option<&str>,
                  incarnation: Option<&str>,
                  qualification: &str,
                  history: Option<&str>| {
        database.execute(
        "INSERT INTO binding_origin_cursors(source_scope,observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id) VALUES(?1,'2026-09-09T00:00:00Z',?2,?3,?4,?5)", params![scope,current,incarnation,qualification,history])
    };
    constraint(cursor(None, None, "unknown", None));
    constraint(cursor(None, None, "unobserved", Some("observation")));
    constraint(cursor(Some("wt"), None, "native_verified", None));
    cursor(None, None, "unobserved", None).unwrap();
    database
        .execute("DELETE FROM binding_origin_cursors", [])
        .unwrap();
    cursor(Some("wt"), None, "unknown", Some("observation")).unwrap();
    assert!(
        database
            .prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
}
