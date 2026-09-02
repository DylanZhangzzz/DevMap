use std::io::Cursor;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use time::OffsetDateTime;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::CommandOutput;
use crate::canonical::canonical_json;
use crate::dock::DockService;
use crate::dock_asset::dock_html;
use crate::error::DevMapError;

const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerHandle {
    pub address: SocketAddr,
    pub token: String,
}

pub struct ViewerRuntime {
    shutdown: Arc<AtomicBool>,
    server: Arc<Server>,
    worker: Option<JoinHandle<Result<(), DevMapError>>>,
}

struct ViewerState {
    dock: DockService,
    last_refresh: Instant,
}

impl ViewerState {
    fn snapshot(&mut self) -> Result<Vec<u8>, DevMapError> {
        if self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.dock.refresh(OffsetDateTime::now_utc())?;
            self.last_refresh = Instant::now();
        }
        canonical_json(self.dock.snapshot())
    }

    fn revision(&self) -> u64 {
        self.dock.snapshot().revision
    }
}

impl ViewerRuntime {
    pub fn shutdown(mut self) -> Result<(), DevMapError> {
        self.shutdown.store(true, Ordering::Release);
        self.server.unblock();
        self.join_worker()
    }

    fn wait(mut self) -> Result<(), DevMapError> {
        self.join_worker()
    }

    fn join_worker(&mut self) -> Result<(), DevMapError> {
        self.worker
            .take()
            .ok_or(DevMapError::ViewerWorker)?
            .join()
            .map_err(|_| DevMapError::ViewerWorker)??;
        Ok(())
    }
}

pub fn start_live_viewer(
    source: &Path,
    bind: SocketAddr,
) -> Result<(ViewerHandle, ViewerRuntime), DevMapError> {
    if !bind.ip().is_loopback() {
        return Err(DevMapError::NonLoopbackViewerBind(bind));
    }
    let dock = DockService::open(source)?;
    let server =
        Arc::new(Server::http(bind).map_err(|error| DevMapError::Viewer(error.to_string()))?);
    let address = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| DevMapError::Viewer("listener did not resolve to an IP address".into()))?;
    let token = random_token()?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let worker_server = Arc::clone(&server);
    let worker_shutdown = Arc::clone(&shutdown);
    let worker_token = token.clone();
    let state = Arc::new(Mutex::new(ViewerState {
        dock,
        last_refresh: Instant::now(),
    }));
    let worker = thread::Builder::new()
        .name("devmap-live-viewer".into())
        .spawn(move || serve(worker_server, worker_shutdown, state, worker_token))?;

    Ok((
        ViewerHandle { address, token },
        ViewerRuntime {
            shutdown,
            server,
            worker: Some(worker),
        },
    ))
}

pub fn run_live(source: &Path) -> Result<CommandOutput, DevMapError> {
    let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let (handle, runtime) = start_live_viewer(source, bind)?;
    println!("http://{}/?token={}", handle.address, handle.token);
    runtime.wait()?;
    Ok(CommandOutput {
        stdout: String::new(),
        exit_code: 0,
    })
}

fn random_token() -> Result<String, DevMapError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| DevMapError::Viewer(error.to_string()))?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut token, "{byte:02x}").map_err(|error| DevMapError::Viewer(error.to_string()))?;
    }
    Ok(token)
}

fn serve(
    server: Arc<Server>,
    shutdown: Arc<AtomicBool>,
    state: Arc<Mutex<ViewerState>>,
    token: String,
) -> Result<(), DevMapError> {
    while !shutdown.load(Ordering::Acquire) {
        if let Some(request) = server.recv_timeout(Duration::from_millis(100))? {
            respond(request, &state, &token)?;
        }
    }
    Ok(())
}

fn respond(
    request: Request,
    state: &Arc<Mutex<ViewerState>>,
    expected_token: &str,
) -> Result<(), DevMapError> {
    let target = request.url().to_owned();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let known = matches!(
        path,
        "/" | "/api/v1/health" | "/api/v1/dock/snapshot" | "/api/v1/dock/events"
    );
    if !known {
        return send(
            request,
            404,
            "text/plain; charset=utf-8",
            b"not found\n".to_vec(),
        );
    }
    if request.method() != &Method::Get {
        return send(
            request,
            405,
            "text/plain; charset=utf-8",
            b"method not allowed\n".to_vec(),
        );
    }
    if query_value(query, "token") != Some(expected_token) {
        return send(
            request,
            401,
            "text/plain; charset=utf-8",
            b"unauthorized\n".to_vec(),
        );
    }

    match path {
        "/" => send(
            request,
            200,
            "text/html; charset=utf-8",
            dock_html().as_bytes().to_vec(),
        ),
        "/api/v1/health" => send(
            request,
            200,
            "application/json",
            br#"{"status":"ok"}"#.to_vec(),
        ),
        "/api/v1/dock/snapshot" => {
            let body = state
                .lock()
                .map_err(|_| DevMapError::Viewer("Dock state lock is poisoned".into()))?
                .snapshot()?;
            send(request, 200, "application/json", body)
        }
        "/api/v1/dock/events" => {
            let after = query_value(query, "after").and_then(|value| value.parse::<u64>().ok());
            let mut state = state
                .lock()
                .map_err(|_| DevMapError::Viewer("Dock state lock is poisoned".into()))?;
            let snapshot = state.snapshot()?;
            let revision = state.revision();
            let body = if after.is_none_or(|value| revision > value) {
                let json = String::from_utf8(snapshot)
                    .map_err(|_| DevMapError::Viewer("Dock snapshot is not UTF-8".into()))?;
                format!("retry: 500\nid: {revision}\nevent: dock\ndata: {json}\n\n")
            } else {
                "retry: 500\n: no newer revision\n\n".to_owned()
            };
            send(request, 200, "text/event-stream", body.into_bytes())
        }
        _ => unreachable!("known route matched above"),
    }
}

fn query_value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then_some(value)
    })
}

fn send(
    request: Request,
    status: u16,
    content_type: &str,
    body: Vec<u8>,
) -> Result<(), DevMapError> {
    let mut response = Response::new(
        StatusCode(status),
        Vec::new(),
        Cursor::new(body.clone()),
        Some(body.len()),
        None,
    );
    for (name, value) in [
        ("Content-Type", content_type),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        (
            "Content-Security-Policy",
            "default-src 'self'; connect-src 'self'; img-src 'self' data:; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    ] {
        response.add_header(
            Header::from_bytes(name.as_bytes(), value.as_bytes())
                .map_err(|_| DevMapError::Viewer(format!("invalid HTTP header: {name}")))?,
        );
    }
    if status == 405 {
        response.add_header(
            Header::from_bytes(b"Allow", b"GET")
                .map_err(|_| DevMapError::Viewer("invalid Allow header".into()))?,
        );
    }
    request
        .respond(response)
        .map_err(|error| DevMapError::Viewer(error.to_string()))
}
