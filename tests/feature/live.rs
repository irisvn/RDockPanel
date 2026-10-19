//! The live streams (server-sent events) through a real server on an
//! ephemeral port: what each sends, and who may open them.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rdockpanel::bootstrap;
use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::docker_service::Docker;
use rustcms::AppState;
use rustcms::auth::AuthState;
use rustcms_testing::{TestApp, TestDb, db_test};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::support::{self, fake_docker::FakeDocker};

const ORIGIN: &str = "http://localhost:8000";

/// The app on `app`'s database, served on a real port until dropped.
struct Server {
    addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

async fn serve(app: &TestApp, docker: FakeDocker) -> Server {
    let mut builder = bootstrap::build(app.state().config().clone());
    builder.bind(Docker::new(docker));
    builder.bind(support::scratch_traefik());
    let served = builder
        .build_with_db(app.state().db().clone())
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(served.serve_on(listener, async move {
        let _ = stopped.await;
    }));
    Server {
        addr,
        stop: Some(stop),
    }
}

/// A session cookie for `user`, as signing in makes it.
async fn cookie(state: &AppState, user: &rustcms::models::user::Model) -> String {
    let auth = state.service::<AuthState>().unwrap();
    let (token, _) =
        rustcms::auth::session::create_for_user(&auth, state.db(), user.id, user.auth_version)
            .await
            .unwrap();
    format!("{}={token}", auth.config.cookie_name)
}

struct Sse {
    status: u16,
    lines: tokio::io::Lines<BufReader<TcpStream>>,
}

async fn open(server: &Server, path: &str, cookie: &str) -> Sse {
    let mut stream = TcpStream::connect(server.addr).await.unwrap();
    let request = format!(
        "GET /admin/api/x{path} HTTP/1.1\r\nHost: localhost:8000\r\nConnection: close\r\nCookie: {cookie}\r\nOrigin: {ORIGIN}\r\nSec-Fetch-Site: same-origin\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut lines = BufReader::new(stream).lines();
    let status_line = lines.next_line().await.unwrap().unwrap_or_default();
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    while let Some(line) = lines.next_line().await.unwrap() {
        if line.trim().is_empty() {
            break;
        }
    }
    Sse { status, lines }
}

impl Sse {
    /// The next event `(name, data)` within `within`.
    async fn next(&mut self, within: Duration) -> Option<(String, serde_json::Value)> {
        let deadline = Instant::now() + within;
        let (mut name, mut data) = (None, String::new());
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let line = tokio::time::timeout(left, self.lines.next_line())
                .await
                .ok()?
                .ok()??;
            let line = line.trim_end_matches('\r');
            if let Some(value) = line.strip_prefix("event:") {
                name = Some(value.trim().to_owned());
            } else if let Some(value) = line.strip_prefix("data:") {
                data.push_str(value.trim_start());
            } else if line.is_empty()
                && let Some(name) = name.take()
            {
                return Some((name, serde_json::from_str(&data).unwrap_or_default()));
            }
        }
    }

    /// The first event named `name` within `within`.
    async fn find(&mut self, name: &str, within: Duration) -> Option<serde_json::Value> {
        let deadline = Instant::now() + within;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let (event, data) = self.next(left).await?;
            if event == name {
                return Some(data);
            }
        }
    }
}

const OPERATOR: &[&str] = &["docker.view", "docker.logs", "docker.manage", "server.view"];

db_test!(
    streams_send_logs_stats_and_samples,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let server = serve(&app, fake).await;
        let cookie = cookie(app.state(), &operator).await;
        let seconds = Duration::from_secs;

        let mut logs = open(&server, "/docker/containers/web/logs/follow/10", &cookie).await;
        assert_eq!(logs.status, 200);
        let lines = logs.find("lines", seconds(5)).await.expect("log lines");
        assert_eq!(lines["lines"][0]["text"], "id-web started\n");
        assert_eq!(lines["lines"][1]["stream"], "stderr");

        let mut stats = open(&server, "/docker/containers/web/stats", &cookie).await;
        assert_eq!(stats.status, 200);
        let reading = stats
            .find("stats", seconds(5))
            .await
            .expect("a stats reading");
        assert_eq!(reading["cpu_percent"], 12.5);

        let mut samples = open(&server, "/server/live", &cookie).await;
        assert_eq!(samples.status, 200);
        let history = samples
            .find("history", seconds(5))
            .await
            .expect("the history");
        assert!(history["samples"].is_array());
        let sample = samples
            .find("sample", seconds(10))
            .await
            .expect("a live sample");
        assert!(sample["mem_total"].as_u64().unwrap_or(0) > 0, "{sample}");
    }
);

db_test!(
    the_panels_own_containers_stream_only_with_system_manage,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let admin = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&admin).await;
        let server = serve(&app, fake).await;
        let operator = cookie(app.state(), &operator).await;
        let admin = cookie(app.state(), &admin).await;

        for path in [
            "/docker/containers/panel/logs/follow",
            "/docker/containers/panel/stats",
        ] {
            assert_eq!(open(&server, path, &operator).await.status, 403, "{path}");
            assert_eq!(open(&server, path, &admin).await.status, 200, "{path}");
        }
        assert_eq!(
            open(&server, "/docker/containers/nope/stats", &operator)
                .await
                .status,
            404
        );
        assert_eq!(
            open(&server, "/docker/containers/..%2Fx/stats", &operator)
                .await
                .status,
            404
        );
    }
);
