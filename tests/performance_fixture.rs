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
