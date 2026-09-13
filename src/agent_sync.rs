//! Read-only Codex desktop adapter. Inventory and runtime activity have separate sources.
//! The desktop IPC protocol is private; unsupported frames never imply inactivity.
use crate::{
    application::ClientQuery, dock::ObservedTask, error::DevMapError, git::SourceWorkspace,
    store::RepositoryStore,
};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Inventory {
    pub observed_at: String,
    pub tasks: Vec<ObservedTask>,
    pub complete: bool,
    pub source: String,
}

pub(crate) struct Subscription(Arc<AtomicBool>);

/// A rebuildable cache must not be mistaken for an interrupted domain migration.
/// Never relax recovery for a shadow containing any domain/provenance records.
pub(crate) fn cache_only(store: &RepositoryStore) -> Result<bool, DevMapError> {
    let c = store.connection();
    let metadata: (String, i64) = c.query_row(
        "SELECT backend_state,generation FROM store_meta WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    if metadata != ("shadow".into(), 0) {
        return Ok(false);
    }
    let tables = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if !tables.iter().any(|t| t == "agent_inventory_cache") {
        return Ok(false);
    }
    for table in tables
        .iter()
        .filter(|t| !matches!(t.as_str(), "store_meta" | "agent_inventory_cache"))
    {
        let count: i64 = c.query_row(
            &format!("SELECT count(*) FROM \"{}\"", table.replace('"', "\"\"")),
            [],
            |r| r.get(0),
        )?;
        if count != 0 {
            return Ok(false);
        }
    }
    let version: Option<i64> = c
        .query_row(
            "SELECT version FROM agent_inventory_cache WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(version == Some(1))
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn start(workspace: SourceWorkspace) -> Option<Subscription> {
    #[cfg(windows)]
    {
        if std::env::var("DEVMAP_AGENT_SYNC").as_deref() == Ok("off") {
            return None;
        }
        let home = std::env::var_os("CODEX_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".codex"))
            })?;
        let database = home.join("state_5.sqlite");
        if std::env::var_os("DEVMAP_AGENT_SYNC_TRACE").is_some() {
            eprintln!("Agent sync database available: {}", database.is_file());
        }
        if !database.is_file() {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        std::thread::Builder::new()
            .name("devmap-agent-sync".into())
            .spawn(move || {
                if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    runtime.block_on(desktop::run(workspace, database, worker_stop));
                }
            })
            .ok()?;
        Some(Subscription(stop))
    }
    #[cfg(not(windows))]
    {
        let _ = workspace;
        None
    }
}

/// Optional, rebuildable cache extension: no domain schema or origin facts are changed.
#[cfg(windows)]
fn save(workspace: &SourceWorkspace, inventory: &Inventory) -> Result<(), DevMapError> {
    let bytes = serde_json::to_string(inventory)?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(DevMapError::Store("Agent inventory too large".into()));
    }
    let mut store = RepositoryStore::open(workspace)?;
    store.transaction(|tx| {
        tx.execute_batch("CREATE TABLE IF NOT EXISTS agent_inventory_cache (singleton INTEGER PRIMARY KEY CHECK(singleton=1), version INTEGER NOT NULL, payload TEXT NOT NULL CHECK(json_valid(payload))) STRICT;")?;
        tx.execute("INSERT INTO agent_inventory_cache VALUES(1,1,?1) ON CONFLICT(singleton) DO UPDATE SET version=1,payload=excluded.payload", [&bytes])?;
        Ok(())
    })
}

/// Every viewer reads the same snapshot. Client-specific working-directory reports remain separate.
pub(crate) fn overlay(
    workspace: &SourceWorkspace,
    query: &mut ClientQuery,
) -> Result<(), DevMapError> {
    let Some(store) = RepositoryStore::open_existing(workspace)? else {
        return Ok(());
    };
    let exists: bool = store.connection().query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='agent_inventory_cache')", [], |r| r.get(0))?;
    if !exists {
        return Ok(());
    }
    let bytes: Option<String> = store
        .connection()
        .query_row(
            "SELECT payload FROM agent_inventory_cache WHERE singleton=1 AND version=1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(bytes) = bytes else {
        return Ok(());
    };
    if bytes.len() > 2 * 1024 * 1024 {
        return Err(DevMapError::Store("Agent cache exceeds limit".into()));
    }
    let inventory: Inventory = serde_json::from_str(&bytes)?;
    if inventory.tasks.len() > 1024
        || time::OffsetDateTime::parse(
            &inventory.observed_at,
            &time::format_description::well_known::Rfc3339,
        )
        .is_err()
    {
        return Err(DevMapError::Store("Invalid Agent cache observation".into()));
    }
    merge(query, inventory);
    Ok(())
}

fn merge(query: &mut ClientQuery, mut inventory: Inventory) {
    let instant = |s: &str| {
        time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
    };
    if query.inventory_observed_at.as_deref().and_then(instant) > instant(&inventory.observed_at) {
        return;
    }
    for task in &mut inventory.tasks {
        if let Some(old) = query
            .tasks
            .iter()
            .find(|old| old.session_id == task.session_id && old.host == task.host)
        {
            task.working_directory = old.working_directory.clone();
            task.subagents = old.subagents.clone();
        }
    }
    if !inventory.complete {
        for old in &query.tasks {
            if !inventory
                .tasks
                .iter()
                .any(|t| t.session_id == old.session_id && t.host == old.host)
            {
                inventory.tasks.push(old.clone());
            }
        }
    }
    query.tasks = inventory.tasks;
    query.complete = inventory.complete;
    query.inventory_observed_at = Some(inventory.observed_at);
}

#[cfg(windows)]
mod desktop {
    use super::*;
    use crate::{dock::TaskLifecycle, presence::PresenceStatus};
    use serde_json::{Value, json};
    use std::{
        collections::{HashMap, HashSet},
        path::{Path, PathBuf},
        time::Duration,
    };
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::windows::named_pipe::{ClientOptions, NamedPipeClient},
    };

    const MAX_FRAME: usize = 32 * 1024 * 1024;
    fn stamp() -> String {
        OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .expect("UTC timestamp")
    }
    fn normalized(p: &str) -> String {
        let p = p.replace('\\', "/").trim_end_matches('/').to_lowercase();
        if let Some(unc) = p.strip_prefix("//?/unc/") {
            format!("//{unc}")
        } else {
            p.strip_prefix("//?/").unwrap_or(&p).to_owned()
        }
    }
    fn sql_inventory(
        workspace: &SourceWorkspace,
        database: &Path,
    ) -> Result<Inventory, DevMapError> {
        let worktrees = crate::worktrees::WorktreeScanner::scan(workspace)?;
        let paths: HashSet<_> = worktrees
            .iter()
            .map(|w| normalized(&w.root.to_string_lossy()))
            .collect();
        read_catalog(database, &paths)
    }

    fn read_catalog(database: &Path, paths: &HashSet<String>) -> Result<Inventory, DevMapError> {
        let connection = rusqlite::Connection::open_with_flags(
            database,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(500))?;
        connection.pragma_update(None, "query_only", true)?;
        // Read metadata only; never load prompts, transcripts, credentials, or tool outputs.
        let mut stmt = connection.prepare("SELECT id,coalesce(name,title),cwd,updated_at,source FROM threads WHERE archived=0 ORDER BY id")?;
        let mut rows = stmt.query([])?;
        let mut tasks = vec![];
        let mut complete = true;
        let mut scanned = 0;
        while let Some(row) = rows.next()? {
            scanned += 1;
            if scanned > 100000 {
                complete = false;
                break;
            }
            let cwd: String = row.get(2)?;
            if !paths.contains(&normalized(&cwd)) {
                continue;
            }
            let source: String = row.get(4)?;
            if serde_json::from_str::<Value>(&source)
                .ok()
                .is_some_and(|v| v.get("subagent").is_some())
            {
                continue;
            }
            if !matches!(source.as_str(), "cli" | "vscode" | "exec" | "appServer") {
                complete = false;
                continue;
            }
            if tasks.len() >= 1024 {
                complete = false;
                break;
            }
            let id: String = row.get(0)?;
            if id.len() != 36 {
                complete = false;
                continue;
            }
            let title: String = row.get(1)?;
            if title.len() > 16384 {
                complete = false;
                continue;
            }
            let updated = OffsetDateTime::from_unix_timestamp(row.get(3)?)
                .map_err(|_| DevMapError::Store("Invalid Codex metadata timestamp".into()))?
                .format(&Rfc3339)?;
            tasks.push(ObservedTask {
                working_directory: None,
                subagents: None,
                lifecycle: TaskLifecycle::Present,
                session_id: id,
                display_title: title,
                host: "local".into(),
                host_status: "unknown".into(),
                workspace_path: cwd,
                status: PresenceStatus::Unknown,
                updated_at: updated,
            });
        }
        Ok(Inventory {
            observed_at: stamp(),
            tasks,
            complete,
            source: "codex-state-v5 + desktop-ipc-v11".into(),
        })
    }

    #[cfg(test)]
    mod catalog_tests {
        use super::*;
        #[test]
        fn runtime_updates_follow_revisions_and_drop_uncertain_activity() {
            let mut states = HashMap::new();
            apply_change(&mut states,"task",&json!({"type":"snapshot","revision":3,"conversationState":{"threadRuntimeStatus":{"type":"active","activeFlags":[]}}})).unwrap();
            apply_change(&mut states,"task",&json!({"type":"patches","baseRevision":3,"revision":4,"patches":[{"op":"replace","path":["threadRuntimeStatus"],"value":{"type":"idle"}}]})).unwrap();
            assert_eq!(states["task"].1["type"], "idle");
            apply_change(
                &mut states,
                "task",
                &json!({"type":"patches","baseRevision":2,"revision":5,"patches":[]}),
            )
            .unwrap();
            assert!(!states.contains_key("task"));
            apply_change(
                &mut states,
                "task",
                &json!({"type":"patches","baseRevision":5,"revision":6,"patches":[]}),
            )
            .unwrap();
            assert!(!states.contains_key("task"));
        }
        #[test]
        fn catalog_tracks_new_renamed_archived_tasks_without_counting_subagents() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("state.sqlite");
            let db = rusqlite::Connection::open(&file).unwrap();
            db.execute_batch("CREATE TABLE threads(id TEXT,name TEXT,title TEXT,cwd TEXT,updated_at INTEGER,source TEXT,archived INTEGER);").unwrap();
            let id = "01a081a1-751a-7473-aa3b-91995c128f7e";
            db.execute(
                "INSERT INTO threads VALUES(?1,NULL,'First','C:/repo',1789344000,'vscode',0)",
                [id],
            )
            .unwrap();
            let paths = HashSet::from([normalized("C:/repo")]);
            let first = read_catalog(&file, &paths).unwrap();
            assert!(first.complete);
            assert_eq!(first.tasks.len(), 1);
            assert_eq!(first.tasks[0].host_status, "unknown");
            db.execute("UPDATE threads SET cwd=?1", [r"\\?\C:\repo"])
                .unwrap();
            assert_eq!(read_catalog(&file, &paths).unwrap().tasks.len(), 1);
            db.execute("UPDATE threads SET name='Renamed'", []).unwrap();
            assert_eq!(
                read_catalog(&file, &paths).unwrap().tasks[0].display_title,
                "Renamed"
            );
            db.execute("INSERT INTO threads SELECT '01a081a1-751a-7473-aa3b-91995c128f7f',NULL,'Child',cwd,updated_at,'{\"subagent\":{}}',0 FROM threads LIMIT 1",[]).unwrap();
            assert_eq!(read_catalog(&file, &paths).unwrap().tasks.len(), 1);
            db.execute("UPDATE threads SET archived=1 WHERE id=?1", [id])
                .unwrap();
            let archived = read_catalog(&file, &paths).unwrap();
            assert!(archived.complete);
            assert!(archived.tasks.is_empty());
            db.execute("UPDATE threads SET archived=0 WHERE id=?1", [id])
                .unwrap();
            assert_eq!(read_catalog(&file, &paths).unwrap().tasks.len(), 1);
            db.execute_batch("ALTER TABLE threads RENAME COLUMN source TO unsupported_source")
                .unwrap();
            assert!(read_catalog(&file, &paths).is_err());
        }
    }

    fn apply_change(
        states: &mut HashMap<String, (u64, Value)>,
        id: &str,
        change: &Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let revision = change["revision"]
            .as_u64()
            .ok_or("Missing desktop revision")?;
        if change["type"] == "snapshot" {
            // Keep activity metadata only; discard message and tool contents immediately.
            states.insert(
                id.to_owned(),
                (
                    revision,
                    change["conversationState"]["threadRuntimeStatus"].clone(),
                ),
            );
        } else if change["type"] == "patches" {
            let Some((previous, status)) = states.get_mut(id) else {
                return Ok(());
            };
            if Some(*previous) != change["baseRevision"].as_u64() {
                states.remove(id);
                return Ok(());
            }
            for patch in change["patches"]
                .as_array()
                .ok_or("Invalid desktop patches")?
            {
                let path = patch["path"]
                    .as_array()
                    .ok_or("Invalid desktop patch path")?;
                if path.first().is_some_and(|v| v == "threadRuntimeStatus") {
                    if path.len() == 1 && patch["op"] != "remove" {
                        *status = patch["value"].clone();
                    } else {
                        *status = Value::Null;
                    }
                }
            }
            *previous = revision;
        }
        Ok(())
    }

    struct Peer {
        socket: NamedPipeClient,
        buffer: Vec<u8>,
        client: String,
        next: u64,
        pending: HashMap<String, String>,
        states: HashMap<String, (u64, Value)>,
        owners: HashMap<String, String>,
        confirmed: HashMap<String, std::time::Instant>,
    }
    impl Peer {
        async fn open() -> Result<Self, Box<dyn std::error::Error>> {
            let socket = ClientOptions::new().open(r"\\.\pipe\codex-ipc")?;
            let mut peer = Self {
                socket,
                buffer: vec![],
                client: "initializing-client".into(),
                next: 0,
                pending: HashMap::new(),
                states: HashMap::new(),
                owners: HashMap::new(),
                confirmed: HashMap::new(),
            };
            peer.request(
                "initialize",
                json!({"clientType":"devmap-agent-observer"}),
                0,
                None,
            )
            .await?;
            Ok(peer)
        }
        async fn write(&mut self, value: Value) -> Result<(), Box<dyn std::error::Error>> {
            let data = serde_json::to_vec(&value)?;
            let mut framed = (data.len() as u32).to_le_bytes().to_vec();
            framed.extend(data);
            tokio::time::timeout(Duration::from_secs(2), self.socket.write_all(&framed)).await??;
            Ok(())
        }
        async fn request(
            &mut self,
            method: &str,
            params: Value,
            version: u64,
            task: Option<String>,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.next += 1;
            let id = format!("devmap-{}-{}", std::process::id(), self.next);
            if let Some(task) = task {
                self.pending.insert(id.clone(), task);
            }
            self.write(json!({"type":"request","requestId":id,"sourceClientId":self.client,"version":version,"method":method,"params":params,"timeoutMs":3000})).await
        }
        async fn follow(
            &mut self,
            task: &str,
            owner: &str,
            following: bool,
        ) -> Result<(), Box<dyn std::error::Error>> {
            self.write(json!({"type":"broadcast","method":"thread-stream-following-changed","sourceClientId":self.client,"targetClientIds":[owner],"version":1,"params":{"hostId":"local","conversationId":task,"following":following}})).await
        }
        async fn reconcile(
            &mut self,
            inventory: &Inventory,
        ) -> Result<(), Box<dyn std::error::Error>> {
            if self.client == "initializing-client" {
                return Ok(());
            }
            let ids: HashSet<_> = inventory
                .tasks
                .iter()
                .map(|t| t.session_id.clone())
                .collect();
            for (id, owner) in self.owners.clone() {
                if !ids.contains(&id) {
                    self.follow(&id, &owner, false).await?;
                    self.owners.remove(&id);
                    self.states.remove(&id);
                }
            }
            self.pending.clear();
            for task in &inventory.tasks {
                self.request(
                    "thread-owner-discovery",
                    json!({"hostId":"local","conversationId":task.session_id}),
                    1,
                    Some(task.session_id.clone()),
                )
                .await?;
            }
            Ok(())
        }
        async fn receive(&mut self) -> Result<bool, Box<dyn std::error::Error>> {
            let mut chunk = [0u8; 65536];
            match tokio::time::timeout(Duration::from_millis(250), self.socket.read(&mut chunk))
                .await
            {
                Err(_) => return Ok(false),
                Ok(Err(e)) => return Err(e.into()),
                Ok(Ok(0)) => return Err("Desktop IPC closed".into()),
                Ok(Ok(n)) => self.buffer.extend_from_slice(&chunk[..n]),
            }
            while self.buffer.len() >= 4 {
                let size = u32::from_le_bytes(self.buffer[..4].try_into()?) as usize;
                if size == 0 || size > MAX_FRAME {
                    return Err("Unsupported desktop IPC frame".into());
                }
                if self.buffer.len() < 4 + size {
                    break;
                }
                let value: Value = serde_json::from_slice(&self.buffer[4..4 + size])?;
                self.buffer.drain(..4 + size);
                self.message(value).await?;
            }
            Ok(true)
        }
        async fn message(&mut self, value: Value) -> Result<(), Box<dyn std::error::Error>> {
            let text =
                |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
            match value["type"].as_str() {
                Some("client-discovery-request")=>self.write(json!({"type":"client-discovery-response","requestId":value["requestId"],"response":{"canHandle":false}})).await?,
                Some("response")=>{
                    if value["method"]=="initialize"&&value["resultType"]=="success" {self.client=text(&value["result"],"clientId");}
                    if let Some(id)=self.pending.remove(&text(&value,"requestId")) {
                        if value["resultType"]=="success" {
                            let owner=text(&value,"handledByClientId");
                            if !owner.is_empty() {
                                self.confirmed.insert(id.clone(),std::time::Instant::now());
                                let changed=self.owners.insert(id.clone(),owner.clone()).as_ref()!=Some(&owner);
                                if changed||self.states.get(&id).is_none_or(|(_,status)|status.is_null()) {self.follow(&id,&owner,true).await?;}
                            }
                        } else {self.states.remove(&id);self.owners.remove(&id);}
                    }
                }
                Some("broadcast") if value["method"]=="thread-stream-state-changed"=>{
                    let p=&value["params"];let id=text(p,"conversationId");
                    if p["hostId"]!="local" || self.owners.get(&id).map(String::as_str) != value["sourceClientId"].as_str() {return Ok(());}
                    if value["version"]!=11 {self.states.remove(&id);return Ok(());}
                    apply_change(&mut self.states,&id,&p["change"])?;
                }
                _=>{}
            }
            Ok(())
        }
        fn apply(&self, inventory: &mut Inventory) {
            for task in &mut inventory.tasks {
                if self
                    .confirmed
                    .get(&task.session_id)
                    .is_none_or(|seen| seen.elapsed() > Duration::from_secs(15))
                {
                    continue;
                }
                if let Some((_, value)) = self.states.get(&task.session_id) {
                    let state = value["type"]
                        .as_str()
                        .or_else(|| value.as_str())
                        .unwrap_or("unknown");
                    let waiting = value["activeFlags"]
                        .as_array()
                        .is_some_and(|flags| !flags.is_empty());
                    let (host, status) = match state {
                        "active" if waiting => ("waiting", PresenceStatus::Waiting),
                        "active" => ("active", PresenceStatus::Working),
                        "idle" => ("idle", PresenceStatus::Idle),
                        "notLoaded" => ("notLoaded", PresenceStatus::Stale),
                        _ => ("unknown", PresenceStatus::Unknown),
                    };
                    task.host_status = host.into();
                    task.status = status;
                }
            }
        }
    }

    pub(super) async fn run(workspace: SourceWorkspace, database: PathBuf, stop: Arc<AtomicBool>) {
        let mut peer: Option<Peer> = None;
        let mut lock = None;
        let mut inventory = None;
        let mut last_saved = String::new();
        let mut next_poll = std::time::Instant::now();
        let mut next_connect = std::time::Instant::now();
        while !stop.load(Ordering::Relaxed) {
            if std::time::Instant::now() >= next_poll {
                next_poll = std::time::Instant::now() + Duration::from_secs(5);
                match crate::git_process::with_operation(|| sql_inventory(&workspace, &database)) {
                    Ok(value) => {
                        if std::env::var_os("DEVMAP_AGENT_SYNC_TRACE").is_some() {
                            eprintln!("Agent sync catalog: {} tasks", value.tasks.len());
                        }
                        // Only a repository with observed tasks needs a cache. Opening a
                        // shadow store does not activate or migrate legacy domain data.
                        if lock.is_none() {
                            if value.tasks.is_empty() {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }
                            if let Err(error) = RepositoryStore::open(&workspace) {
                                if std::env::var_os("DEVMAP_AGENT_SYNC_TRACE").is_some() {
                                    eprintln!("Agent sync initialize: {error}");
                                }
                                continue;
                            }
                            let path = workspace
                                .git_common_dir
                                .join("devmap")
                                .join("agent-sync.lock");
                            if let Ok(file) = std::fs::OpenOptions::new()
                                .create(true)
                                .truncate(false)
                                .read(true)
                                .write(true)
                                .open(path)
                            {
                                if fs2::FileExt::try_lock_exclusive(&file).is_ok() {
                                    lock = Some(file);
                                } else {
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                    continue;
                                }
                            }
                            if lock.is_none() {
                                continue;
                            }
                        }
                        inventory = Some(value);
                        if let (Some(p), Some(v)) = (&mut peer, &inventory)
                            && p.reconcile(v).await.is_err()
                        {
                            peer = None;
                        }
                    }
                    Err(error) => {
                        if std::env::var_os("DEVMAP_AGENT_SYNC_TRACE").is_some() {
                            eprintln!("Agent sync catalog: {error}");
                        }
                        inventory = None;
                    }
                }
            }
            if lock.is_some() && peer.is_none() && std::time::Instant::now() >= next_connect {
                peer = Peer::open().await.ok();
                next_connect = std::time::Instant::now() + Duration::from_secs(5);
            }
            if let Some(p) = &mut peer {
                if p.receive().await.is_err() {
                    peer = None;
                }
            } else {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if let Some(value) = &inventory {
                let mut value = value.clone();
                if let Some(p) = &peer {
                    p.apply(&mut value);
                }
                // Preserve the catalog observation timestamp; rendering is never a fresh host observation.
                let fingerprint = serde_json::to_string(&value).unwrap_or_default();
                if fingerprint != last_saved {
                    match save(&workspace, &value) {
                        Ok(()) => last_saved = fingerprint,
                        Err(error) => {
                            eprintln!("DevMap Agent cache update failed: {error}");
                            inventory = None;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        dock::{TaskLifecycle, WorkingDirectoryObservation},
        presence::PresenceStatus,
    };
    fn task(id: &str) -> ObservedTask {
        ObservedTask {
            session_id: id.into(),
            display_title: id.into(),
            host: "local".into(),
            host_status: "unknown".into(),
            workspace_path: "C:/repo".into(),
            updated_at: "2026-09-13T00:00:00Z".into(),
            lifecycle: TaskLifecycle::Present,
            status: PresenceStatus::Unknown,
            working_directory: None,
            subagents: None,
        }
    }
    #[cfg(windows)]
    #[test]
    fn independent_readers_reopen_one_sqlite_inventory_without_domain_writes() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet", "-b", "main"])
                .arg(dir.path())
                .status()
                .unwrap()
                .success()
        );
        let workspace = crate::git::SourceGitInspector::open(dir.path())
            .unwrap()
            .workspace_allow_unborn()
            .unwrap();
        let input = Inventory {
            tasks: vec![task("cached")],
            observed_at: "2026-09-13T01:00:00Z".into(),
            complete: true,
            source: "fixture".into(),
        };
        save(&workspace, &input).unwrap();
        for _ in 0..2 {
            let mut q = ClientQuery {
                tasks: vec![],
                inventory_observed_at: None,
                complete: false,
                previous_heads: vec![],
            };
            overlay(&workspace, &mut q).unwrap();
            assert_eq!(q.tasks[0].session_id, "cached");
            assert_eq!(
                q.inventory_observed_at.as_deref(),
                Some(input.observed_at.as_str())
            );
        }
        let store = RepositoryStore::open_existing(&workspace).unwrap().unwrap();
        let state: (String, i64) = store
            .connection()
            .query_row("SELECT backend_state,generation FROM store_meta", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(state, ("shadow".into(), 0));
        assert!(cache_only(&store).unwrap());
        assert!(matches!(
            crate::store::migration::prepare_first_origin_write(&workspace).unwrap(),
            crate::store::migration::WriteBackend::LegacyPreserved(_)
        ));
        assert_eq!(
            store
                .connection()
                .query_row("SELECT count(*) FROM journal_records", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        drop(store);
        let mut changed = RepositoryStore::open(&workspace).unwrap();
        changed
            .transaction(|tx| {
                tx.execute(
                    "INSERT INTO binding_watermarks VALUES('fixture','2026-09-13T00:00:00Z','{}')",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        assert!(!cache_only(&changed).unwrap());
        assert!(crate::store::migration::prepare_first_origin_write(&workspace).is_err());
    }
    #[test]
    fn full_inventory_removes_archived_tasks_but_partial_does_not() {
        let mut q = ClientQuery {
            tasks: vec![task("a"), task("b")],
            inventory_observed_at: None,
            complete: false,
            previous_heads: vec![],
        };
        let i = Inventory {
            tasks: vec![task("a")],
            observed_at: "2026-09-13T01:00:00Z".into(),
            complete: false,
            source: "test".into(),
        };
        merge(&mut q, i.clone());
        assert_eq!(q.tasks.len(), 2);
        merge(
            &mut q,
            Inventory {
                complete: true,
                ..i
            },
        );
        assert_eq!(q.tasks.len(), 1);
    }
    #[test]
    fn polling_does_not_renew_or_clear_explicit_working_directory_reports() {
        let mut t = task("a");
        let report = WorkingDirectoryObservation {
            path: "C:/repo/worktree".into(),
            observed_at: "2026-09-13T00:00:00Z".into(),
            source: "agent_report".into(),
        };
        t.working_directory = Some(report.clone());
        let mut q = ClientQuery {
            tasks: vec![t],
            inventory_observed_at: None,
            complete: false,
            previous_heads: vec![],
        };
        merge(
            &mut q,
            Inventory {
                tasks: vec![task("a")],
                observed_at: "2026-09-13T01:00:00Z".into(),
                complete: true,
                source: "test".into(),
            },
        );
        assert_eq!(q.tasks[0].working_directory, Some(report));
    }
}
