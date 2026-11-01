#[allow(dead_code)]
mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_nats::Subscriber;
use bytes::Bytes;
use common::{TestNats, TestNatsOpts, new_test_nats};
use futures::StreamExt;
use naps::config::{Connection, From, Mode, Route, To};
use naps::connect::connect;
use naps::metrics::Metrics;
use naps::pipeline::run_route;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const RECV_TIMEOUT: Duration = Duration::from_secs(10);
const SILENCE_TIMEOUT: Duration = Duration::from_millis(500);

struct TestPipeline {
    _nats: TestNats,
    client: async_nats::Client,
    cancel: CancellationToken,
    route: JoinHandle<anyhow::Result<()>>,
}

fn new_test_route(processor: Option<&str>, to_subject: &str) -> Route {
    Route {
        name: "test".to_string(),
        from: From::Subject {
            subject: "in.*".to_string(),
            queue_group: "naps-test".to_string(),
        },
        processor: processor.map(|name| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/scripts")
                .join(name)
        }),
        to: To {
            mode: Mode::Core,
            subject: Some(to_subject.to_string()),
        },
    }
}

async fn new_test_client(nats: &TestNats) -> async_nats::Client {
    let conn = Connection {
        url: nats.url.clone(),
        tls: None,
        auth: None,
    };
    connect(&conn, "naps-test").await.expect("connect")
}

async fn wait_route_ready(ready: oneshot::Receiver<()>) {
    tokio::time::timeout(RECV_TIMEOUT, ready)
        .await
        .expect("route ready in time")
        .expect("route ready");
}

async fn new_test_pipeline(processor: Option<&str>, to_subject: &str) -> TestPipeline {
    let nats = new_test_nats(TestNatsOpts::default()).await;
    let client = new_test_client(&nats).await;
    let cancel = CancellationToken::new();
    let (ready, ready_rx) = oneshot::channel();
    let route = tokio::spawn(run_route(
        new_test_route(processor, to_subject),
        client.clone(),
        client.clone(),
        cancel.clone(),
        ready,
        Arc::new(Metrics::new()),
    ));
    wait_route_ready(ready_rx).await;
    TestPipeline {
        _nats: nats,
        client,
        cancel,
        route,
    }
}

async fn subscribe_flushed(client: &async_nats::Client, subject: &'static str) -> Subscriber {
    let subscriber = client.subscribe(subject).await.expect("subscribe");
    client.flush().await.expect("flush");
    subscriber
}

async fn publish_payload(client: &async_nats::Client, subject: &'static str, payload: String) {
    client
        .publish(subject, Bytes::from(payload))
        .await
        .expect("publish");
}

async fn next_payload(subscriber: &mut Subscriber, timeout: Duration) -> Option<String> {
    tokio::time::timeout(timeout, async {
        let message = subscriber.next().await.expect("subscription open");
        String::from_utf8(message.payload.to_vec()).expect("utf8 payload")
    })
    .await
    .ok()
}

#[tokio::test]
async fn test_relay_without_processor() {
    let pipeline = new_test_pipeline(None, "out.a").await;
    let mut out = subscribe_flushed(&pipeline.client, "out.a").await;

    publish_payload(&pipeline.client, "in.a", "hello".to_string()).await;

    assert_eq!(
        next_payload(&mut out, RECV_TIMEOUT).await.as_deref(),
        Some("hello")
    );
}

#[tokio::test]
async fn test_processor_subject_wins_over_to_subject() {
    let pipeline = new_test_pipeline(Some("pipeline_subject.ts"), "out.ignored").await;
    let mut out_c = subscribe_flushed(&pipeline.client, "out.c").await;
    let mut ignored = subscribe_flushed(&pipeline.client, "out.ignored").await;

    publish_payload(&pipeline.client, "in.a", "hello".to_string()).await;

    assert_eq!(
        next_payload(&mut out_c, RECV_TIMEOUT).await.as_deref(),
        Some("hello")
    );
    assert_eq!(next_payload(&mut ignored, SILENCE_TIMEOUT).await, None);
}

#[tokio::test]
async fn test_processor_fan_out() {
    let pipeline = new_test_pipeline(Some("pipeline_fan_out.ts"), "out.a").await;
    let mut out = subscribe_flushed(&pipeline.client, "out.a").await;

    publish_payload(&pipeline.client, "in.a", "hello".to_string()).await;

    assert_eq!(
        next_payload(&mut out, RECV_TIMEOUT).await.as_deref(),
        Some("hello")
    );
    assert_eq!(
        next_payload(&mut out, RECV_TIMEOUT).await.as_deref(),
        Some("hello")
    );
    assert_eq!(next_payload(&mut out, SILENCE_TIMEOUT).await, None);
}

#[tokio::test]
async fn test_processor_null_delivers_nothing() {
    let pipeline = new_test_pipeline(Some("pipeline_drop.ts"), "out.a").await;
    let mut out = subscribe_flushed(&pipeline.client, "out.a").await;

    publish_payload(&pipeline.client, "in.a", "hello".to_string()).await;

    assert_eq!(next_payload(&mut out, SILENCE_TIMEOUT).await, None);
}

#[tokio::test]
async fn test_queue_group_delivers_each_message_once() {
    let pipeline = new_test_pipeline(None, "out.a").await;
    let (ready, ready_rx) = oneshot::channel();
    let second = tokio::spawn(run_route(
        new_test_route(None, "out.a"),
        pipeline.client.clone(),
        pipeline.client.clone(),
        pipeline.cancel.clone(),
        ready,
        Arc::new(Metrics::new()),
    ));
    wait_route_ready(ready_rx).await;
    let mut out = subscribe_flushed(&pipeline.client, "out.a").await;

    for i in 0..10 {
        publish_payload(&pipeline.client, "in.a", format!("m{i}")).await;
    }

    let mut received = Vec::new();
    while let Some(payload) = next_payload(&mut out, SILENCE_TIMEOUT).await {
        received.push(payload);
    }
    received.sort();
    let mut expected: Vec<String> = (0..10).map(|i| format!("m{i}")).collect();
    expected.sort();
    assert_eq!(received, expected);

    pipeline.cancel.cancel();
    second.await.expect("join").expect("second route");
}

#[tokio::test]
async fn test_cancel_stops_route() {
    let pipeline = new_test_pipeline(None, "out.a").await;

    pipeline.cancel.cancel();

    let result = pipeline.route.await.expect("join");
    assert!(result.is_ok(), "unexpected error: {result:?}");
}

#[tokio::test]
async fn test_startup_failure_returns_err_without_ready() {
    let nats = new_test_nats(TestNatsOpts::default()).await;
    let client = new_test_client(&nats).await;
    let (ready, ready_rx) = oneshot::channel();

    let result = run_route(
        new_test_route(Some("no_recv.js"), "out.a"),
        client.clone(),
        client.clone(),
        CancellationToken::new(),
        ready,
        Arc::new(Metrics::new()),
    )
    .await;

    assert!(result.is_err(), "expected startup error");
    assert!(ready_rx.await.is_err(), "ready must not fire");
}
