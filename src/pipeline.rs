use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{SyncSender, sync_channel};

use anyhow::{Context, anyhow, bail};
use async_nats::HeaderMap;
use futures::StreamExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use crate::config::{From, Mode, Route};
use crate::msg::Msg;
use crate::processor::{Output, new_processor};

const PROCESSOR_QUEUE_SIZE: usize = 64;

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
    let From::Subject {
        subject,
        queue_group,
    } = &route.from
    else {
        bail!("jetstream not supported yet");
    };
    if route.to.mode == Mode::Jetstream {
        bail!("jetstream not supported yet");
    }

    let processor = match &route.processor {
        Some(path) => Some(spawn_processor(&route.name, path).await?),
        None => None,
    };

    let mut subscription = source
        .queue_subscribe(subject.clone(), queue_group.clone())
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
                if let Err(err) = relay(&route, processor.as_ref(), &destination, msg).await {
                    warn!(route = route.name, error = format!("{err:#}"), "message dropped");
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
    destination: &async_nats::Client,
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
        destination
            .publish_with_headers(
                subject,
                new_header_map(&output.msg.headers),
                output.msg.data,
            )
            .await
            .context("publish")?;
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
