#[allow(dead_code)]
mod common;

use std::path::PathBuf;
use std::time::Duration;

use async_nats::Subscriber;
use async_nats::jetstream::{self, consumer::PullConsumer, consumer::pull};
use bytes::Bytes;
use common::{TestNats, TestNatsOpts, new_test_nats};
use futures::StreamExt;
use naps::config::{Connection, From, Mode, Route, To};
use naps::connect::connect;
use naps::pipeline::run_route;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const RECV_TIMEOUT: Duration = Duration::from_secs(10);
const SILENCE_TIMEOUT: Duration = Duration::from_millis(500);
const IN_STREAM: &str = "IN";
const IN_CONSUMER: &str = "naps-in";
const OUT_STREAM: &str = "OUT";
const OUT_CONSUMER: &str = "out-reader";

struct TestJetstream {
    _nats: TestNats,
    client: async_nats::Client,
    context: jetstream::Context,
}

struct TestPipeline {
    js: TestJetstream,
    in_consumer: PullConsumer,
    cancel: CancellationToken,
    route: JoinHandle<anyhow::Result<()>>,
}

fn new_test_route(from: From, processor: Option<&str>, mode: Mode, to_subject: &str) -> Route {
    Route {
        name: "test".to_string(),
        from,
        processor: processor.map(|name| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/scripts")
                .join(name)
        }),
        to: To {
            mode,
            subject: Some(to_subject.to_string()),
        },
    }
}

fn new_test_jetstream_from(consumer: &str) -> From {
    From::Jetstream {
        stream: IN_STREAM.to_string(),
        consumer: consumer.to_string(),
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

async fn new_test_jetstream() -> TestJetstream {
    let nats = new_test_nats(TestNatsOpts {
        jetstream: true,
        ..TestNatsOpts::default()
    })
    .await;
    let client = new_test_client(&nats).await;
    let context = jetstream::new(client.clone());
    TestJetstream {
        _nats: nats,
        client,
        context,
    }
}

async fn new_test_stream(js: &TestJetstream, name: &str, subjects: &str) {
    js.context
        .create_stream(jetstream::stream::Config {
            name: name.to_string(),
            subjects: vec![subjects.to_string()],
            ..Default::default()
        })
        .await
        .expect("create stream");
}

async fn new_test_consumer(js: &TestJetstream, stream: &str, durable: &str) -> PullConsumer {
    js.context
        .create_consumer_on_stream(
            pull::Config {
                durable_name: Some(durable.to_string()),
                ..Default::default()
            },
            stream,
        )
        .await
        .expect("create consumer")
}

async fn wait_route_ready(ready: oneshot::Receiver<()>) {
    tokio::time::timeout(RECV_TIMEOUT, ready)
        .await
        .expect("route ready in time")
        .expect("route ready");
}

async fn new_test_pipeline(processor: Option<&str>) -> TestPipeline {
    let js = new_test_jetstream().await;
    new_test_stream(&js, IN_STREAM, "in.*").await;
    let in_consumer = new_test_consumer(&js, IN_STREAM, IN_CONSUMER).await;
    let cancel = CancellationToken::new();
    let (ready, ready_rx) = oneshot::channel();
    let route = tokio::spawn(run_route(
        new_test_route(
            new_test_jetstream_from(IN_CONSUMER),
            processor,
            Mode::Core,
            "out.a",
        ),
        js.client.clone(),
        js.client.clone(),
        cancel.clone(),
        ready,
    ));
    wait_route_ready(ready_rx).await;
    TestPipeline {
        js,
        in_consumer,
        cancel,
        route,
    }
}

async fn subscribe_flushed(client: &async_nats::Client, subject: &'static str) -> Subscriber {
    let subscriber = client.subscribe(subject).await.expect("subscribe");
    client.flush().await.expect("flush");
    subscriber
}

async fn publish_payload(js: &TestJetstream, subject: &'static str, payload: String) {
    js.context
        .publish(subject, Bytes::from(payload))
        .await
        .expect("publish")
        .await
        .expect("publish ack");
}

async fn next_payload(subscriber: &mut Subscriber, timeout: Duration) -> Option<String> {
    tokio::time::timeout(timeout, async {
        let message = subscriber.next().await.expect("subscription open");
        String::from_utf8(message.payload.to_vec()).expect("utf8 payload")
    })
    .await
    .ok()
}

async fn wait_delivered_and_acked(consumer: &mut PullConsumer) {
    tokio::time::timeout(RECV_TIMEOUT, async {
        loop {
            let info = consumer.info().await.expect("consumer info");
            if info.delivered.consumer_sequence >= 1 && info.num_ack_pending == 0 {
                return;
            }
        }
    })
    .await
    .expect("input acked in time");
}

#[tokio::test]
async fn test_jetstream_source_delivers_and_acks() {
    let mut pipeline = new_test_pipeline(None).await;
    let mut out = subscribe_flushed(&pipeline.js.client, "out.a").await;

    publish_payload(&pipeline.js, "in.a", "hello".to_string()).await;

    assert_eq!(
        next_payload(&mut out, RECV_TIMEOUT).await.as_deref(),
        Some("hello")
    );
    wait_delivered_and_acked(&mut pipeline.in_consumer).await;
    assert_eq!(pipeline.in_consumer.cached_info().num_ack_pending, 0);
    pipeline.cancel.cancel();
    pipeline.route.await.expect("join").expect("route");
}

#[tokio::test]
async fn test_jetstream_source_acks_zero_outputs() {
    let mut pipeline = new_test_pipeline(Some("pipeline_drop.ts")).await;

    publish_payload(&pipeline.js, "in.a", "hello".to_string()).await;

    wait_delivered_and_acked(&mut pipeline.in_consumer).await;
    assert_eq!(pipeline.in_consumer.cached_info().num_ack_pending, 0);
    pipeline.cancel.cancel();
    pipeline.route.await.expect("join").expect("route");
}

#[tokio::test]
async fn test_jetstream_source_naks_on_processing_error() {
    let mut pipeline = new_test_pipeline(Some("jetstream_fail_once.ts")).await;
    let mut out = subscribe_flushed(&pipeline.js.client, "out.a").await;

    publish_payload(&pipeline.js, "in.a", "hello".to_string()).await;

    assert_eq!(
        next_payload(&mut out, RECV_TIMEOUT).await.as_deref(),
        Some("hello")
    );
    assert_eq!(next_payload(&mut out, SILENCE_TIMEOUT).await, None);
    wait_delivered_and_acked(&mut pipeline.in_consumer).await;
    assert!(
        pipeline
            .in_consumer
            .cached_info()
            .delivered
            .consumer_sequence
            >= 2
    );
    pipeline.cancel.cancel();
    pipeline.route.await.expect("join").expect("route");
}

#[tokio::test]
async fn test_jetstream_destination_stores_message() {
    let js = new_test_jetstream().await;
    new_test_stream(&js, OUT_STREAM, "out.*").await;
    let out_consumer = new_test_consumer(&js, OUT_STREAM, OUT_CONSUMER).await;
    let cancel = CancellationToken::new();
    let (ready, ready_rx) = oneshot::channel();
    let route = tokio::spawn(run_route(
        new_test_route(
            From::Subject {
                subject: "in.*".to_string(),
                queue_group: "naps-test".to_string(),
            },
            None,
            Mode::Jetstream,
            "out.a",
        ),
        js.client.clone(),
        js.client.clone(),
        cancel.clone(),
        ready,
    ));
    wait_route_ready(ready_rx).await;

    js.client
        .publish("in.a", Bytes::from("hello"))
        .await
        .expect("publish");

    let mut messages = out_consumer.messages().await.expect("consumer messages");
    let stored = tokio::time::timeout(RECV_TIMEOUT, messages.next())
        .await
        .expect("stored in time")
        .expect("stream open")
        .expect("message");
    assert_eq!(stored.message.payload, Bytes::from("hello"));
    assert!(
        tokio::time::timeout(SILENCE_TIMEOUT, messages.next())
            .await
            .is_err()
    );
    cancel.cancel();
    route.await.expect("join").expect("route");
}

#[tokio::test]
async fn test_missing_consumer_returns_err_without_ready() {
    let js = new_test_jetstream().await;
    new_test_stream(&js, IN_STREAM, "in.*").await;
    let (ready, ready_rx) = oneshot::channel();

    let result = run_route(
        new_test_route(
            new_test_jetstream_from("missing"),
            None,
            Mode::Core,
            "out.a",
        ),
        js.client.clone(),
        js.client.clone(),
        CancellationToken::new(),
        ready,
    )
    .await;

    let err = result.expect_err("expected startup error");
    assert!(format!("{err:#}").contains("missing"), "{err:#}");
    assert!(ready_rx.await.is_err(), "ready must not fire");
}
