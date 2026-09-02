mod support;

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use devmap::error::DevMapError;
use devmap::viewer::start_live_viewer;
use serde_json::Value;

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
    assert_eq!(model["schema_version"], "devmap/dock/1");
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
