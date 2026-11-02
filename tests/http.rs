#[allow(dead_code)]
mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use bytes::Bytes;
use common::{TestNats, TestNatsOpts, new_test_nats};
use futures::StreamExt;
use naps::config::{Connection, From, Mode, Route, To};
use naps::connect::connect;
use naps::http::{HttpState, serve};
use naps::metrics::Metrics;
use naps::pipeline::run_route;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const RECV_TIMEOUT: Duration = Duration::from_secs(10);

struct TestHttp {
    _nats: TestNats,
    client: async_nats::Client,
    base_url: String,
    metrics: Arc<Metrics>,
    draining: Arc<AtomicBool>,
    http: reqwest::Client,
}

fn new_test_route(processor: Option<&str>) -> Route {
    Route {
        name: "t".to_string(),
        from: From::Subject {
            subject: "in.*".to_string(),
            queue_group: "naps-t".to_string(),
        },
        processor: processor.map(|name| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/scripts")
                .join(name)
        }),
        to: To {
            mode: Mode::Core,
            subject: Some("out.a".to_string()),
        },
    }
}

async fn new_test_http() -> TestHttp {
    let nats = new_test_nats(TestNatsOpts::default()).await;
    let conn = Connection {
        url: nats.url.clone(),
        tls: None,
        auth: None,
    };
    let client = connect(&conn, "naps-test").await.expect("connect");
    let metrics = Arc::new(Metrics::new());
    let draining = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
    let state = HttpState {
        source: client.clone(),
        destination: client.clone(),
        metrics: metrics.clone(),
        draining: draining.clone(),
    };
    tokio::spawn(serve(listener, state, CancellationToken::new()));
    TestHttp {
        _nats: nats,
        client,
        base_url,
        metrics,
        draining,
        http: reqwest::Client::new(),
    }
}

async fn start_test_route(test: &TestHttp, processor: Option<&str>) {
    let (ready, ready_rx) = oneshot::channel();
    tokio::spawn(run_route(
        new_test_route(processor),
        test.client.clone(),
        test.client.clone(),
        CancellationToken::new(),
        ready,
        test.metrics.clone(),
    ));
    tokio::time::timeout(RECV_TIMEOUT, ready_rx)
        .await
        .expect("route ready in time")
        .expect("route ready");
}

async fn get_status(test: &TestHttp, path: &str) -> u16 {
    test.http
        .get(format!("{}{path}", test.base_url))
        .send()
        .await
        .expect("get")
        .status()
        .as_u16()
}

async fn get_metrics(test: &TestHttp) -> String {
    test.http
        .get(format!("{}/metrics", test.base_url))
        .send()
        .await
        .expect("get metrics")
        .text()
        .await
        .expect("metrics body")
}

async fn wait_metrics_contain(test: &TestHttp, needle: &str) -> String {
    tokio::time::timeout(RECV_TIMEOUT, async {
        loop {
            let body = get_metrics(test).await;
            if body.contains(needle) {
                return body;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("metrics never contained {needle}"))
}

#[tokio::test]
async fn test_healthz_returns_ok() {
    let test = new_test_http().await;

    assert_eq!(get_status(&test, "/healthz").await, 200);
}

#[tokio::test]
async fn test_readyz_returns_ok_after_connect() {
    let test = new_test_http().await;

    assert_eq!(get_status(&test, "/readyz").await, 200);
}

#[tokio::test]
async fn test_readyz_returns_unavailable_while_draining() {
    let test = new_test_http().await;

    test.draining.store(true, Ordering::SeqCst);

    assert_eq!(get_status(&test, "/readyz").await, 503);
}

#[tokio::test]
async fn test_metrics_count_relayed_message() {
    let test = new_test_http().await;
    start_test_route(&test, None).await;
    let mut out = test.client.subscribe("out.a").await.expect("subscribe");
    test.client.flush().await.expect("flush");

    test.client
        .publish("in.a", Bytes::from("hello"))
        .await
        .expect("publish");
    tokio::time::timeout(RECV_TIMEOUT, out.next())
        .await
        .expect("relayed in time")
        .expect("subscription open");

    let body = wait_metrics_contain(&test, "naps_messages_out_total{route=\"t\"} 1").await;
    assert!(
        body.contains("naps_messages_in_total{route=\"t\"} 1"),
        "{body}"
    );
}

#[tokio::test]
async fn test_metrics_count_process_error() {
    let test = new_test_http().await;
    start_test_route(&test, Some("throws.ts")).await;

    test.client
        .publish("in.a", Bytes::from("hello"))
        .await
        .expect("publish");
    wait_metrics_contain(&test, "naps_errors_total{route=\"t\",stage=\"process\"} 1").await;
}
