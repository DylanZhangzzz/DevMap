//! Explicit synthetic scale corpus; never runs in the ordinary test suite.
mod support;
use devmap::{
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::SourceGitInspector,
    journal::JournalStore,
    presence::{PresenceSignal, PresenceStore},
};
use serde_json::json;
use std::{fs, path::PathBuf};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn count(name: &str, default: usize, limit: usize) -> usize {
    let n = std::env::var(name)
        .map(|s| s.parse().expect("positive integer scale"))
        .unwrap_or(default);
    assert!((1..=limit).contains(&n));
    n
}

#[test]
#[ignore = "explicit synthetic performance fixture generation, not a performance pass"]
fn generate_disposable_legacy_scale_corpus() {
    let worktree_count = count("DEVMAP_SCALE_WORKTREES", 20, 64);
    let session_count = count("DEVMAP_SCALE_SESSIONS", 100, 2048);
    let events_per_session = count("DEVMAP_SCALE_EVENTS", 1000, 10000);
    assert!(session_count * events_per_session <= 1_000_000);
    let verification = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/verification");
    fs::create_dir_all(&verification).unwrap();
    let holder = tempfile::Builder::new()
        .prefix("scale-legacy-")
        .tempdir_in(&verification)
        .unwrap();
    let root = holder.path().join("main");
    fs::create_dir(&root).unwrap();
    support::git(&root, ["init", "-b", "main"]);
    support::git(&root, ["config", "user.name", "Synthetic scale fixture"]);
    support::git(&root, ["config", "user.email", "fixture@example.invalid"]);
    support::git(
        &root,
        ["commit", "--allow-empty", "-m", "Scale fixture base"],
    );
    let mut paths = vec![root.clone()];
    for n in 1..worktree_count {
        let path = holder.path().join(format!("worktree-{n}"));
        support::git(
            &root,
            [
                "worktree",
                "add",
                "-b",
                &format!("codex/scale-{n}"),
                path.to_str().unwrap(),
            ],
        );
        paths.push(path);
    }
    let workspaces: Vec<_> = paths
        .iter()
        .map(|p| SourceGitInspector::open(p).unwrap().workspace().unwrap())
        .collect();
    let presence: Vec<_> = workspaces
        .iter()
        .map(|w| PresenceStore::open(w).unwrap())
        .collect();
    let now = OffsetDateTime::now_utc();
    let stamp = now.format(&Rfc3339).unwrap();
    for n in 0..session_count {
        let slot = n % worktree_count;
        let w = &workspaces[slot];
        let session = format!("019a0000-0000-7000-8000-{:012x}", n + 1);
        let journal = JournalStore::open(w, &session).unwrap();
        let records = journal
            .append_batch_with(|start| {
                (0..events_per_session)
                    .map(|index| {
                        EventEnvelope::new(
                            EVENT_SCHEMA_VERSION,
                            format!("scale-{n}-{index}"),
                            EventType::ToolCompleted,
                            start + index as u64,
                            &stamp,
                            HostIdentity::new("synthetic_scale", "1")?,
                            ActorIdentity::new(format!("agent-{n}"), None)?,
                            SessionContext::new(
                                &session,
                                None,
                                w.root.to_string_lossy(),
                                Some(w.root.to_string_lossy().into_owned()),
                                w.branch.clone(),
                                Some(w.head.clone()),
                            )?,
                            json!({"capture_grade":"D","activity":"tool_completed"}),
                        )
                    })
                    .collect()
            })
            .unwrap();
        presence[slot]
            .observe(PresenceSignal::AcceptedRecords(&records), now)
            .unwrap();
    }
    assert!(
        !workspaces[0]
            .git_common_dir
            .join("devmap/devmap.db")
            .exists()
    );
    let manifest = json!({"scope":"synthetic_legacy_scale_fixture","source":root,"worktrees":paths,"sessions":session_count,"events_per_session":events_per_session,"events":session_count*events_per_session,"evaluation_time":stamp,"note":"Generated through domain APIs with synthetic events; not real host evidence and not a performance result."});
    fs::write(
        holder.path().join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let retained = holder.keep();
    println!("SCALE_FIXTURE {}", retained.join("manifest.json").display());
}

#[test]
#[ignore = "explicit sealed allocation only; retains failures; not performance acceptance"]
fn generate_owned_schema2_scale_corpus() {
    owned_schema2::generate();
}

mod owned_schema2 {
    use super::*;
    use std::{io::Write, path::Path};
    const RESERVE: u64 = 512 * 1024 * 1024;
    const LATER_LOGS: u64 = 128 * 1024 * 1024;

    fn checked(path: &Path) -> PathBuf {
        assert!(path.is_absolute());
        for parent in path.ancestors() {
            let metadata = fs::symlink_metadata(parent).unwrap();
            assert!(!metadata.file_type().is_symlink());
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                assert_eq!(
                    metadata.file_attributes() & 0x400,
                    0,
                    "reparse path refused"
                );
            }
        }
        fs::canonicalize(path).unwrap()
    }

    #[cfg(windows)]
    fn physical(path: &Path) -> String {
        use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(path)
            .unwrap();
        let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: File retains the handle and info is valid writable storage.
        assert_ne!(
            unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) },
            0
        );
        // SAFETY: a successful call initialized info.
        let info = unsafe { info.assume_init() };
        format!(
            "windows:{}:{}",
            info.dwVolumeSerialNumber,
            (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
        )
    }
    #[cfg(unix)]
    fn physical(path: &Path) -> String {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path).unwrap();
        format!("unix:{}:{}", metadata.dev(), metadata.ino())
    }

    fn save_new(path: &Path, value: &serde_json::Value) {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(&serde_json::to_vec_pretty(value).unwrap())
            .unwrap();
        file.sync_all().unwrap();
    }

    struct Allocation {
        root: PathBuf,
        canonical: PathBuf,
        physical: String,
        receipt: PathBuf,
        seal: String,
        log: fs::File,
    }
    impl Allocation {
        fn verify(&self) {
            assert_eq!(checked(&self.root), self.canonical);
            assert_eq!(physical(&self.root), self.physical);
            assert_eq!(
                devmap::canonical::sha256_hex(&fs::read(&self.receipt).unwrap()),
                self.seal
            );
        }
        fn stage(&mut self, name: &str, extra: serde_json::Value) {
            self.verify();
            let free = fs2::available_space(&self.root).unwrap();
            assert!(
                free >= RESERVE + LATER_LOGS,
                "disk reserve exhausted; retain allocation"
            );
            let value = json!({"stage":name,"available_bytes":free,"at":OffsetDateTime::now_utc().format(&Rfc3339).unwrap(),"details":extra});
            writeln!(self.log, "{}", value).unwrap();
            self.log.sync_all().unwrap();
            println!("OWNED_SCHEMA2_STAGE {value}");
        }
        fn budget(&self, payload: u64) {
            self.verify();
            assert!(
                fs2::available_space(&self.root).unwrap()
                    >= RESERVE + LATER_LOGS + payload.checked_mul(8).unwrap(),
                "measured payload exceeds conservative retained corpus+SQL/WAL budget"
            );
        }
    }

    pub(super) fn generate() {
        let receipt = PathBuf::from(
            std::env::var_os("DEVMAP_SCHEMA2_ALLOCATION_RECEIPT")
                .expect("sealed Node allocation required"),
        );
        let seal =
            std::env::var("DEVMAP_SCHEMA2_ALLOCATION_SHA256").expect("allocation seal required");
        let bytes = fs::read(&receipt).unwrap();
        assert_eq!(devmap::canonical::sha256_hex(&bytes), seal);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema"], "devmap/schema2-allocation/1");
        let root = PathBuf::from(value["native_root"].as_str().unwrap());
        let canonical = checked(&root);
        assert_eq!(
            checked(Path::new(value["allocation_root"].as_str().unwrap())),
            canonical
        );
        assert_eq!(checked(receipt.parent().unwrap()), canonical);
        assert_eq!(
            physical(&root),
            value["physical_identity"].as_str().unwrap()
        );
        let entries: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![receipt.file_name().unwrap().to_owned()],
            "allocation is not pristine; no resume/adoption"
        );
        let log = fs::OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(root.join("rust-stages.ndjson"))
            .unwrap();
        let mut allocation = Allocation {
            physical: physical(&root),
            root,
            canonical,
            receipt,
            seal,
            log,
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            populate(&mut allocation, &value)
        }));
        if let Err(error) = result {
            // Failure evidence does not assert that reserve/identity is still valid.
            let _ = writeln!(
                allocation.log,
                "{}",
                json!({"stage":"failed","note":"Rust generation panicked; artifacts retained, no recovery attempted"})
            );
            let _ = allocation.log.sync_all();
            std::panic::resume_unwind(error);
        }
    }

    fn populate(a: &mut Allocation, receipt: &serde_json::Value) {
        let dims = &receipt["dimensions"];
        let worktree_count = dims["worktrees"].as_u64().unwrap() as usize;
        let session_count = dims["sessions"].as_u64().unwrap() as usize;
        let per_session = dims["events_per_session"].as_u64().unwrap() as usize;
        if receipt["mode"] == "scale" {
            assert_eq!(
                (worktree_count, session_count, per_session),
                (20, 100, 1000)
            );
        } else {
            assert_eq!(receipt["mode"], "smoke");
            assert!((1..=4).contains(&worktree_count));
            assert!((1..=8).contains(&session_count));
            assert!((1..=20).contains(&per_session));
        }
        assert!(session_count >= worktree_count && session_count.is_multiple_of(worktree_count));
        let total = session_count.checked_mul(per_session).unwrap();
        a.budget(total as u64 * 1024 + 8192);
        a.stage("preparing", json!({"dimensions":dims}));
        let root = a.root.join("main");
        fs::create_dir(&root).unwrap();
        support::git(&root, ["init", "-b", "main"]);
        support::git(&root, ["config", "user.name", "Synthetic schema2 fixture"]);
        support::git(&root, ["config", "user.email", "fixture@example.invalid"]);
        let probe = root.join("freshness-probe.txt");
        fs::write(&probe, b"owned synthetic schema2 freshness probe\n").unwrap();
        support::git(&root, ["add", "freshness-probe.txt"]);
        support::git(&root, ["commit", "-m", "Synthetic schema2 base"]);
        let mut paths = vec![root.clone()];
        for n in 1..worktree_count {
            a.verify();
            let path = a.root.join(format!("w{n}"));
            support::git(
                &root,
                [
                    "worktree",
                    "add",
                    "-b",
                    &format!("codex/s2-{n}"),
                    path.to_str().unwrap(),
                ],
            );
            paths.push(path);
        }
        let workspaces: Vec<_> = paths
            .iter()
            .map(|p| SourceGitInspector::open(p).unwrap().workspace().unwrap())
            .collect();
        let stamp = receipt["evaluation_time"].as_str().unwrap();
        let now = OffsetDateTime::parse(stamp, &Rfc3339).unwrap();
        let mut sessions = Vec::new();
        let mut payload = 0u64;
        for n in 0..session_count {
            a.verify();
            let w = &workspaces[n % worktree_count];
            let session = format!("019b0000-0000-7000-8000-{:012x}", n + 1);
            let journal = JournalStore::open(w, &session).unwrap();
            let records = journal
                .append_batch_with(|start| {
                    (0..per_session)
                        .map(|index| {
                            let actor = if n == 0 && index + 1 == per_session {
                                "语料🙂".repeat(500)
                            } else {
                                format!("a{n}")
                            };
                            EventEnvelope::new(
                                EVENT_SCHEMA_VERSION,
                                format!("s2-{n}-{index}"),
                                EventType::ToolCompleted,
                                start + index as u64,
                                stamp,
                                HostIdentity::new("synthetic_scale", "2")?,
                                ActorIdentity::new(actor, None)?,
                                SessionContext::new(
                                    &session,
                                    None,
                                    w.root.to_string_lossy(),
                                    Some(w.root.to_string_lossy().into_owned()),
                                    w.branch.clone(),
                                    Some(w.head.clone()),
                                )?,
                                json!({"capture_grade":"D","activity":"tool_completed"}),
                            )
                        })
                        .collect()
                })
                .unwrap();
            for (index, record) in records.iter().enumerate() {
                let size = serde_json::to_vec(record).unwrap().len() + 1;
                assert!(
                    size <= if n == 0 && index + 1 == per_session {
                        16384
                    } else {
                        1024
                    },
                    "pilot record exceeds planned bound; retain and re-budget"
                );
                payload += size as u64;
            }
            PresenceStore::open(w)
                .unwrap()
                .observe(PresenceSignal::AcceptedRecords(&records), now)
                .unwrap();
            assert_eq!(journal.replay().unwrap(), records);
            assert_eq!(records.len(), per_session);
            assert_eq!(records.last().unwrap().sequence, per_session as u64);
            sessions.push(json!({"session_id":session,"records":records.len(),"last_sha256":records.last().unwrap().sha256,"worktree_id":devmap::worktrees::WorktreeScanner::scan(w).unwrap().into_iter().find(|r|r.is_current).unwrap().worktree_id}));
            if n == 0 {
                a.budget(payload * session_count as u64);
                a.stage("pilot_verified",json!({"records":records.len(),"serialized_bytes":payload,"projected_payload":payload*session_count as u64}));
            }
            assert!(fs2::available_space(&a.root).unwrap() >= RESERVE + LATER_LOGS);
        }
        let main = &workspaces[0];
        let db = main.git_common_dir.join("devmap/devmap.db");
        assert!(!db.exists());
        a.stage(
            "legacy_complete",
            json!({"serialized_records_bytes":payload,"sessions":session_count,"events":total}),
        );
        a.budget(payload);
        let snapshot = a.root.join("frozen-baseline");
        let frozen = devmap::store::migration::freeze(main, &snapshot, now).unwrap();
        a.stage("frozen", json!({"manifest_files":frozen.files.len()}));
        devmap::store::migration::import_shadow(main, &snapshot).unwrap();
        a.stage("imported", json!({}));
        let pair = devmap::store::migration::compare_snapshot(main, &snapshot, &[]).unwrap();
        assert_eq!(
            serde_json::to_value(&pair.legacy).unwrap(),
            serde_json::to_value(&pair.sql).unwrap()
        );
        drop(pair);
        devmap::store::migration::activate(main, &snapshot).unwrap();
        let verified = devmap::store::migration::verify(main).unwrap();
        assert!(verified.verified);
        assert_eq!(verified.backend, "active");
        let c =
            rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        assert_eq!(
            c.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        assert!(
            c.prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query([])
                .unwrap()
                .next()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            c.query_row("SELECT schema_version FROM store_meta", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            2
        );
        let rows = devmap::worktrees::WorktreeScanner::scan(main).unwrap();
        let current = rows.iter().find(|r| r.is_current).unwrap();
        let manifest = json!({"schema":"devmap-synthetic-scale/2","state":"active_verified","allocation_nonce":receipt["nonce"],"source":root,"common":main.git_common_dir,"database":db,"immutable_roots":[snapshot],
            "repository_id":devmap::worktrees::repository_id(main),"current_worktree_id":current.worktree_id,
            "worktrees":rows.iter().map(|r|json!({"root":r.root,"git_dir":r.git_dir,"worktree_id":r.worktree_id,"root_physical":physical(&r.root),"admin_physical":physical(&r.git_dir),"head":r.head})).collect::<Vec<_>>(),
            "dimensions":{"worktrees":worktree_count,"sessions":session_count,"events":total},"events_per_session":per_session,"evaluation_time":stamp,"serialized_records_bytes":payload,"sessions":sessions,
            "change_probe":{"path":probe,"sha256":devmap::canonical::sha256_hex(&fs::read(&probe).unwrap()),"worktree_id":current.worktree_id},
            "generation":verified.generation,"integrity_check":"ok","foreign_key_check_rows":0,"generator_allocation_receipt_sha256":a.seal});
        a.verify();
        save_new(&a.root.join("manifest.json"), &manifest);
        a.stage("active_verified", json!({"manifest":"manifest.json"}));
    }
}
