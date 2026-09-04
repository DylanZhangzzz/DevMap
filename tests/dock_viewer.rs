mod support;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use devmap::dock::ObservedTask;
use devmap::error::DevMapError;
use devmap::presence::PresenceStatus;
use devmap::viewer::{start_live_viewer, start_live_viewer_with_tasks};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

fn request(address: SocketAddr, method: &str, target: &str) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

fn observed_task(workspace: &std::path::Path, title: &str) -> ObservedTask {
    ObservedTask {
        session_id: "01a00000-0000-7000-8000-000000000001".into(),
        display_title: title.into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: workspace.to_string_lossy().into_owned(),
        status: PresenceStatus::Working,
        updated_at: "2026-09-03T10:00:00Z".into(),
    }
}

#[test]
fn viewer_applies_renamed_and_cleared_task_inventory() {
    let repo = support::committed_repo();
    let (handle, runtime) = start_live_viewer_with_tasks(
        repo.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
        vec![observed_task(repo.path(), "Old title")],
    )
    .unwrap();
    let snapshot_path = format!("/api/v1/dock/snapshot?token={}", handle.token);
    let original: Value =
        serde_json::from_str(body(&request(handle.address, "GET", &snapshot_path))).unwrap();
    let original_revision = original["revision"].as_u64().unwrap();

    runtime
        .replace_observed_tasks(
            vec![observed_task(repo.path(), "New title")],
            OffsetDateTime::parse("2026-09-03T10:02:00Z", &Rfc3339).unwrap(),
        )
        .unwrap();
    let renamed: Value =
        serde_json::from_str(body(&request(handle.address, "GET", &snapshot_path))).unwrap();
    assert!(renamed["revision"].as_u64().unwrap() > original_revision);
    assert_eq!(
        renamed["branch_groups"][0]["lanes"][0]["chats"][0]["display_title"],
        "New title"
    );

    runtime
        .replace_observed_tasks(
            Vec::new(),
            OffsetDateTime::parse("2026-09-03T10:03:00Z", &Rfc3339).unwrap(),
        )
        .unwrap();
    let cleared: Value =
        serde_json::from_str(body(&request(handle.address, "GET", &snapshot_path))).unwrap();
    assert!(
        cleared["branch_groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["lanes"].as_array().unwrap())
            .all(|lane| lane["chats"].as_array().unwrap().is_empty())
    );

    runtime.shutdown().unwrap();
}

#[test]
fn viewer_is_loopback_token_protected_read_only_and_stoppable() {
    let repo = support::committed_repo();
    let before = support::source_snapshot(repo.path());
    let (handle, runtime) = start_live_viewer(
        repo.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    )
    .unwrap();
    assert!(handle.address.ip().is_loopback());
    assert_ne!(handle.address.port(), 0);
    assert_eq!(handle.token.len(), 64);
    assert!(handle.token.bytes().all(|byte| byte.is_ascii_hexdigit()));

    let missing = request(handle.address, "GET", "/api/v1/health");
    assert!(missing.starts_with("HTTP/1.1 401"), "{missing}");
    let wrong = request(handle.address, "GET", "/api/v1/health?token=wrong");
    assert!(wrong.starts_with("HTTP/1.1 401"), "{wrong}");

    let health = request(
        handle.address,
        "GET",
        &format!("/api/v1/health?token={}", handle.token),
    );
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.contains("Cache-Control: no-store"));

    let snapshot = request(
        handle.address,
        "GET",
        &format!("/api/v1/dock/snapshot?token={}", handle.token),
    );
    assert!(snapshot.starts_with("HTTP/1.1 200"), "{snapshot}");
    assert!(snapshot.contains("Content-Type: application/json"));
    assert!(snapshot.contains("Cache-Control: no-store"));
    let model: Value = serde_json::from_str(body(&snapshot)).unwrap();
    assert_eq!(model["schema_version"], "devmap/dock/3");
    let revision = model["revision"].as_u64().unwrap();

    let events = request(
        handle.address,
        "GET",
        &format!("/api/v1/dock/events?token={}&after=0", handle.token),
    );
    assert!(events.starts_with("HTTP/1.1 200"), "{events}");
    assert!(events.contains("Content-Type: text/event-stream"));
    assert!(events.contains("event: dock\n"));
    assert!(events.contains(&format!("id: {revision}\n")));
    let event_data = body(&events)
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(event_data).unwrap()["revision"],
        revision
    );

    let replay = request(
        handle.address,
        "GET",
        &format!(
            "/api/v1/dock/events?token={}&after={revision}",
            handle.token
        ),
    );
    assert!(replay.starts_with("HTTP/1.1 200"), "{replay}");
    assert!(!body(&replay).contains("event: dock"));

    for method in ["POST", "PUT", "DELETE"] {
        let response = request(
            handle.address,
            method,
            &format!("/api/v1/dock/snapshot?token={}", handle.token),
        );
        assert!(response.starts_with("HTTP/1.1 405"), "{response}");
    }
    for target in ["/missing", "/../../README.md", "/api/v1/dock/../secret"] {
        let response = request(
            handle.address,
            "GET",
            &format!("{target}?token={}", handle.token),
        );
        assert!(response.starts_with("HTTP/1.1 404"), "{response}");
    }

    assert_eq!(support::source_snapshot(repo.path()), before);
    runtime.shutdown().unwrap();
    assert!(TcpStream::connect_timeout(&handle.address, Duration::from_millis(300)).is_err());
}

#[test]
fn viewer_refuses_non_loopback_bind_and_cli_exposes_only_explicit_live_mode() {
    let repo = support::committed_repo();
    let error = match start_live_viewer(
        repo.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
    ) {
        Err(error) => error,
        Ok(_) => panic!("non-loopback Viewer bind was accepted"),
    };
    assert!(matches!(error, DevMapError::NonLoopbackViewerBind(_)));

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args(["view", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--live"));
    assert!(help.contains("--source"));
    assert!(matches!(
        devmap::run(["devmap", "view", "--source", repo.path().to_str().unwrap()]),
        Err(DevMapError::UnsupportedCommand("canonical topology viewer"))
    ));
}

#[test]
fn browser_shell_reuses_the_dock_asset_without_embedded_credentials() {
    let html = devmap::dock_asset::dock_html();
    assert!(html.contains("location.search"));
    assert!(html.contains("/api/v1/dock/snapshot"));
    assert!(html.contains("/api/v1/dock/events"));
    assert!(!html.contains("0123456789abcdef0123456789abcdef"));
}

#[test]
fn transient_dock_failure_does_not_kill_the_health_endpoint() {
    let repo = support::committed_repo();
    let (handle, runtime) = start_live_viewer(
        repo.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    )
    .unwrap();
    let presence = repo.path().join(".git/devmap/presence");
    std::fs::create_dir_all(&presence).unwrap();
    std::fs::write(presence.join("v1"), b"not-a-directory").unwrap();
    std::thread::sleep(Duration::from_millis(550));

    let failed = request(
        handle.address,
        "GET",
        &format!("/api/v1/dock/snapshot?token={}", handle.token),
    );
    assert!(failed.starts_with("HTTP/1.1 503"), "{failed}");
    let health = request(
        handle.address,
        "GET",
        &format!("/api/v1/health?token={}", handle.token),
    );
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    runtime.shutdown().unwrap();
}
