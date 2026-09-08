use devmap::runtime::protocol::{Hello, HelloReply, VERSION, Welcome};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    _temp: tempfile::TempDir,
    exe: PathBuf,
    repo: PathBuf,
    owner: Option<Child>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(child) = self.owner.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
fn git(path: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let exe = temp.path().join(if cfg!(windows) {
            "devmap.exe"
        } else {
            "devmap"
        });
        fs::copy(env!("CARGO_BIN_EXE_devmap"), &exe).unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--allow-empty",
                "-qm",
                "initial",
            ],
        );
        let owner = Command::new(&exe)
            .args(["runtime", "--owner", "--source"])
            .arg(&repo)
            .args([
                "--instance",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "--idle-seconds",
                "60",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            _temp: temp,
            exe,
            repo,
            owner: Some(owner),
        }
    }
    fn welcome(&self) -> Welcome {
        let out = Command::new(&self.exe)
            .args(["runtime", "--identity", "--source"])
            .arg(&self.repo)
            .output()
            .unwrap();
        assert!(out.status.success());
        let identity: Value = serde_json::from_slice(&out.stdout).unwrap();
        Welcome {
            protocol: VERSION,
            repository: identity["repository"].as_str().unwrap().into(),
            build: format!("{:x}", Sha256::digest(fs::read(&self.exe).unwrap())),
            owner_instance: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            owner_pid: self.owner.as_ref().unwrap().id(),
            client_instance: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        }
    }
}
#[cfg(windows)]
type Stream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(unix)]
type Stream = tokio::net::UnixStream;
async fn connect(f: &Fixture, w: &Welcome) -> Stream {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut s = loop {
        #[cfg(windows)]
        let result = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(format!(r"\\.\pipe\devmap-{}", w.repository));
        #[cfg(unix)]
        let result = {
            let user = format!(
                "{:x}",
                Sha256::digest(unsafe { libc::geteuid() }.to_string().as_bytes())
            );
            let path = fs::canonicalize("/tmp")
                .unwrap()
                .join(format!("devmap-runtime-{}", &user[..16]))
                .join(&w.repository[..32])
                .join("ipc");
            tokio::net::UnixStream::connect(path).await
        };
        match result {
            Ok(s) => break s,
            Err(e) => {
                assert!(std::time::Instant::now() < deadline, "owner startup: {e}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    };
    send(
        &mut s,
        &Hello {
            protocol: VERSION,
            repository: w.repository.clone(),
            build: format!("{:x}", Sha256::digest(fs::read(&f.exe).unwrap())),
            source: fs::canonicalize(&f.repo).unwrap(),
            git_dir: fs::canonicalize(git(&f.repo, &["rev-parse", "--absolute-git-dir"])).unwrap(),
            client_instance: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        },
    )
    .await;
    let reply: HelloReply = serde_json::from_value(receive(&mut s).await).unwrap();
    match reply {
        HelloReply::Accepted { welcome } => {
            assert_eq!(welcome.owner_instance, w.owner_instance);
            assert_eq!(welcome.owner_pid, w.owner_pid);
        }
        other => panic!("{other:?}"),
    }
    s
}
async fn send(s: &mut Stream, value: &impl serde::Serialize) {
    let bytes = serde_json::to_vec(value).unwrap();
    assert!(bytes.len() <= 16384);
    s.write_u32(bytes.len() as u32).await.unwrap();
    s.write_all(&bytes).await.unwrap();
}
async fn receive(s: &mut Stream) -> Value {
    tokio::time::timeout(Duration::from_secs(30), async {
        let n = s.read_u32().await.unwrap() as usize;
        assert!(n <= 16384);
        let mut bytes = vec![0; n];
        s.read_exact(&mut bytes).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    })
    .await
    .unwrap()
}
fn empty_query() -> Value {
    json!({"tasks":[], "inventory_observed_at":null, "complete":false, "previous_heads":[]})
}
#[test]
fn real_owner_query_uses_multipart_and_does_not_create_database() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        let bytes = serde_json::to_vec(&json!({"operation":"Query", "query":empty_query()})).unwrap();
        send(&mut s, &json!({"operation":"Begin", "protocol":VERSION, "repository":w.repository, "client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "request_id":1, "owner_instance":w.owner_instance, "total":bytes.len(), "digest":format!("{:x}",Sha256::digest(&bytes))})).await;
        let ready = receive(&mut s).await; assert_eq!(ready["status"], "Ready");
    });
    assert!(!f.repo.join(".git/devmap/devmap.db").exists());
}

async fn begin(s: &mut Stream, w: &Welcome, request_id: u64, bytes: &[u8]) -> Value {
    let tag = json!({"request_id":request_id,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":format!("{:x}",Sha256::digest(bytes))});
    send(s, &json!({"operation":"Begin", "protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "request_id":request_id,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":tag["digest"]})).await;
    assert_eq!(receive(s).await["status"], "Ready");
    tag
}
async fn call(s: &mut Stream, w: &Welcome, id: u64, request: Value) -> Value {
    let bytes = serde_json::to_vec(&request).unwrap();
    let tag = begin(s, w, id, &bytes).await;
    for (index, bytes) in bytes.chunks(2048).enumerate() {
        send(
            s,
            &json!({"transfer":tag,"offset":index*2048,"bytes":bytes}),
        )
        .await;
    }
    let reply = receive(s).await;
    assert_eq!(reply["status"], "Result", "{reply}");
    let result_tag = &reply["transfer"];
    assert_eq!(result_tag["request_id"], id);
    assert_eq!(result_tag["owner_instance"], w.owner_instance);
    let total = result_tag["total"].as_u64().unwrap() as usize;
    assert!(total <= devmap::runtime::protocol::MAX_RESULT);
    let mut result = vec![];
    while result.len() < total {
        let chunk = receive(s).await;
        assert_eq!(&chunk["transfer"], result_tag);
        assert_eq!(chunk["offset"], result.len());
        result.extend(serde_json::from_value::<Vec<u8>>(chunk["bytes"].clone()).unwrap());
    }
    assert_eq!(result.len(), total);
    assert_eq!(
        result_tag["digest"],
        format!("{:x}", Sha256::digest(&result))
    );
    serde_json::from_slice(&result).unwrap()
}
#[test]
fn complete_query_and_large_inventory_round_trip_keep_original_timestamp() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        let result = call(&mut s, &w, 1, json!({"operation":"Query","query":empty_query()})).await;
        assert_eq!(result["result"], "Snapshot"); assert_eq!(result["snapshot"]["model"]["schema_version"], "devmap/dock/4", "{result}");
        assert!(!f.repo.join(".git/devmap/devmap.db").exists());
        let stamp = time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap();
        let tasks: Vec<_> = (0..100).map(|i| json!({"working_directory":null,"subagents":null,"lifecycle":"present","session_id":format!("task-{i}"),"display_title":"multi-part inventory title".repeat(5),"host":"codex","host_status":"running","workspace_path":f.repo.to_string_lossy(),"status":"working","updated_at":stamp})).collect();
        let request = json!({"operation":"AcceptInventory","prior":empty_query(),"tasks":tasks,"complete":true,"observed_at":stamp});
        assert!(serde_json::to_vec(&request).unwrap().len() > 16384);
        let result = call(&mut s, &w, 2, request).await;
        assert_eq!(result["result"], "Inventory", "{result}");
        assert_eq!(result["query"]["inventory_observed_at"], stamp);
        assert_eq!(result["query"]["tasks"].as_array().unwrap().len(),100);
        let projection = call(&mut s, &w, 3, json!({"operation":"Query","query":result["query"]})).await;
        assert_eq!(projection["result"],"Snapshot");
    });
}
#[test]
fn partial_upload_reserves_capacity_and_disconnect_does_not_harm_peer() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut a = connect(&f,&w).await; let mut b = connect(&f,&w).await; let mut c = connect(&f,&w).await;
        let bytes = serde_json::to_vec(&json!({"operation":"AcceptInventory","prior":empty_query(),"tasks":[],"complete":true,"observed_at":"2026-09-08T00:00:00Z"})).unwrap();
        begin(&mut a,&w,1,&bytes).await; begin(&mut b,&w,1,&bytes).await;
        send(&mut c,&json!({"operation":"Begin","protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_id":1,"owner_instance":w.owner_instance,"total":bytes.len(),"digest":format!("{:x}",Sha256::digest(&bytes))})).await;
        assert_eq!(receive(&mut c).await["status"],"Busy");
        drop(a); drop(b);
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = call(&mut c,&w,2,json!({"operation":"Query","query":empty_query()})).await;
        assert_eq!(result["result"],"Snapshot");
    });
    assert!(
        !f.repo.join(".git/devmap").exists(),
        "partial commands must not write legacy or SQL"
    );
}
#[test]
fn malformed_transfer_offsets_totals_and_digests_never_dispatch() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        for mode in ["offset","total","digest","content"] {
            let mut s = connect(&f,&w).await;
            let bytes = serde_json::to_vec(&json!({"operation":"AcceptInventory","prior":empty_query(),"tasks":[],"complete":true,"observed_at":"2026-09-08T00:00:00Z"})).unwrap();
            let mut tag = begin(&mut s,&w,1,&bytes).await;
            if mode=="total" { tag["total"]=json!(bytes.len()+1); }
            if mode=="digest" { tag["digest"]=json!("0".repeat(64)); }
            let mut content = bytes.clone(); if mode=="content" { content[0]=b' '; }
            send(&mut s,&json!({"transfer":tag,"offset":if mode=="offset" {1}else{0},"bytes":content})).await;
            let rejected = tokio::time::timeout(Duration::from_secs(5),s.read_u32()).await.unwrap();
            assert!(rejected.is_err(),"malformed {mode} accepted");
        }
    });
    assert!(!f.repo.join(".git/devmap").exists());
}

#[test]
fn oversized_begin_is_rejected_before_upload_or_domain_write() {
    let f = Fixture::new();
    let w = f.welcome();
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(async {
        let mut s = connect(&f, &w).await;
        send(&mut s, &json!({"operation":"Begin","protocol":VERSION,"repository":w.repository,"client_instance":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","request_id":1,"owner_instance":w.owner_instance,"total":devmap::runtime::protocol::MAX_REQUEST+1,"digest":"0".repeat(64)})).await;
        assert!(tokio::time::timeout(Duration::from_secs(5),s.read_u32()).await.unwrap().is_err());
    });
    assert!(!f.repo.join(".git/devmap").exists());
}
