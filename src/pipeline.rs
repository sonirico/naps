use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{SyncSender, sync_channel};

use anyhow::{Context, anyhow, bail};
use async_nats::HeaderMap;
use async_nats::jetstream::{self, AckKind, consumer::pull};
use futures::StreamExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::config::{From, Mode, Route};
use crate::msg::Msg;
use crate::processor::{Output, new_processor};

const PROCESSOR_QUEUE_SIZE: usize = 64;

struct Destination {
    core: async_nats::Client,
    jetstream: jetstream::Context,
}

struct ProcessorRequest {
    msg: Msg,
    reply: oneshot::Sender<anyhow::Result<Vec<Output>>>,
}

pub async fn run_route(
    route: Route,
    source: async_nats::Client,
    destination: async_nats::Client,
    cancel: CancellationToken,
    ready: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    let processor = match &route.processor {
        Some(path) => Some(spawn_processor(&route.name, path).await?),
        None => None,
    };
    let destination = Destination {
        core: destination.clone(),
        jetstream: jetstream::new(destination),
    };

    match &route.from {
        From::Subject {
            subject,
            queue_group,
        } => {
            consume_subject(
                &route,
                subject,
                queue_group,
                processor.as_ref(),
                &source,
                &destination,
                cancel,
                ready,
            )
            .await
        }
        From::Jetstream { stream, consumer } => {
            consume_jetstream(
                &route,
                stream,
                consumer,
                processor.as_ref(),
                &source,
                &destination,
                cancel,
                ready,
            )
            .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn consume_subject(
    route: &Route,
    subject: &str,
    queue_group: &str,
    processor: Option<&SyncSender<ProcessorRequest>>,
    source: &async_nats::Client,
    destination: &Destination,
    cancel: CancellationToken,
    ready: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    let mut subscription = source
        .queue_subscribe(subject.to_string(), queue_group.to_string())
        .await
        .with_context(|| format!("subscribe route {} to {subject}", route.name))?;

    source
        .flush()
        .await
        .with_context(|| format!("flush subscription for route {}", route.name))?;
    if ready.send(()).is_err() {
        debug!(route = %route.name, "ready receiver dropped");
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                subscription.unsubscribe().await.context("unsubscribe")?;
                return Ok(());
            }
            next = subscription.next() => {
                let Some(message) = next else {
                    bail!("subscription closed for route {}", route.name);
                };
                let msg = new_msg(message);
                if let Err(err) = relay(route, processor, destination, msg).await {
                    warn!(route = route.name, error = format!("{err:#}"), "message dropped");
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn consume_jetstream(
    route: &Route,
    stream: &str,
    consumer: &str,
    processor: Option<&SyncSender<ProcessorRequest>>,
    source: &async_nats::Client,
    destination: &Destination,
    cancel: CancellationToken,
    ready: oneshot::Sender<()>,
) -> anyhow::Result<()> {
    let bound = jetstream::new(source.clone())
        .get_stream(stream)
        .await
        .with_context(|| format!("get stream {stream} for route {}", route.name))?
        .get_consumer::<pull::Config>(consumer)
        .await
        .map_err(|err| anyhow!(err))
        .with_context(|| {
            format!(
                "get consumer {consumer} of stream {stream} for route {}",
                route.name
            )
        })?;
    let mut messages = bound.messages().await.with_context(|| {
        format!(
            "consume consumer {consumer} of stream {stream} for route {}",
            route.name
        )
    })?;
    if ready.send(()).is_err() {
        debug!(route = %route.name, "ready receiver dropped");
    }

    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            next = messages.next() => {
                let Some(next) = next else {
                    bail!("message stream closed for route {}", route.name);
                };
                let message = match next {
                    Ok(message) => message,
                    Err(err) => {
                        warn!(route = route.name, error = format!("{err:#}"), "message stream error");
                        continue;
                    }
                };
                let msg = new_msg(message.message.clone());
                let kind = match relay(route, processor, destination, msg).await {
                    Ok(()) => AckKind::Ack,
                    Err(err) => {
                        warn!(route = route.name, error = format!("{err:#}"), "message nacked");
                        AckKind::Nak(None)
                    }
                };
                if let Err(err) = message.ack_with(kind).await {
                    warn!(route = route.name, error = format!("{err:#}"), "ack failed");
                }
            }
        }
    }
}

async fn spawn_processor(name: &str, path: &Path) -> anyhow::Result<SyncSender<ProcessorRequest>> {
    let (tx, rx) = sync_channel::<ProcessorRequest>(PROCESSOR_QUEUE_SIZE);
    let (started_tx, started_rx) = oneshot::channel::<anyhow::Result<()>>();
    let path = path.to_path_buf();
    let route = name.to_string();

    std::thread::Builder::new()
        .name(format!("processor-{name}"))
        .spawn(move || {
            let mut processor = match new_processor(&path) {
                Ok(processor) => processor,
                Err(err) => {
                    if started_tx.send(Err(err)).is_err() {
                        warn!(route, "processor startup result dropped");
                    }
                    return;
                }
            };
            if started_tx.send(Ok(())).is_err() {
                return;
            }
            while let Ok(request) = rx.recv() {
                let result = processor.process(request.msg);
                if request.reply.send(result).is_err() {
                    warn!(route, "processor result dropped");
                }
            }
        })
        .context("spawn processor thread")?;

    started_rx
        .await
        .map_err(|_| anyhow!("processor thread exited during startup"))?
        .with_context(|| format!("start processor for route {name}"))?;
    Ok(tx)
}

async fn relay(
    route: &Route,
    processor: Option<&SyncSender<ProcessorRequest>>,
    destination: &Destination,
    msg: Msg,
) -> anyhow::Result<()> {
    let outputs = match processor {
        Some(requests) => process(requests, msg).await?,
        None => vec![Output {
            msg,
            subject_set: false,
        }],
    };

    for output in outputs {
        let subject = match (&route.to.subject, output.subject_set) {
            (Some(to_subject), false) => to_subject.clone(),
            _ => output.msg.subject,
        };
        let headers = new_header_map(&output.msg.headers);
        match route.to.mode {
            Mode::Core => destination
                .core
                .publish_with_headers(subject, headers, output.msg.data)
                .await
                .context("publish")?,
            Mode::Jetstream => {
                destination
                    .jetstream
                    .publish_with_headers(subject, headers, output.msg.data)
                    .await
                    .context("publish")?
                    .await
                    .context("publish ack")?;
            }
        }
    }
    Ok(())
}

async fn process(requests: &SyncSender<ProcessorRequest>, msg: Msg) -> anyhow::Result<Vec<Output>> {
    let (reply, result) = oneshot::channel();
    requests
        .send(ProcessorRequest { msg, reply })
        .map_err(|_| anyhow!("processor thread stopped"))?;
    result
        .await
        .map_err(|_| anyhow!("processor thread stopped"))?
        .context("process")
}

fn new_msg(message: async_nats::Message) -> Msg {
    let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(header_map) = &message.headers {
        for (name, values) in header_map.iter() {
            headers.insert(
                name.to_string(),
                values.iter().map(ToString::to_string).collect(),
            );
        }
    }
    Msg {
        subject: message.subject.to_string(),
        headers,
        data: message.payload,
    }
}

fn new_header_map(headers: &BTreeMap<String, Vec<String>>) -> HeaderMap {
    let mut header_map = HeaderMap::new();
    for (name, values) in headers {
        for value in values {
            header_map.append(name.as_str(), value.as_str());
        }
    }
    header_map
}
