use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::histogram::{Histogram, exponential_buckets};
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct RouteLabels {
    pub route: String,
}

pub const STAGE_PROCESS: &str = "process";
pub const STAGE_PUBLISH: &str = "publish";

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct ErrorLabels {
    pub route: String,
    pub stage: &'static str,
}

pub struct Metrics {
    pub messages_in: Family<RouteLabels, Counter>,
    pub messages_out: Family<RouteLabels, Counter>,
    pub messages_dropped: Family<RouteLabels, Counter>,
    pub errors: Family<ErrorLabels, Counter>,
    pub process_duration: Family<RouteLabels, Histogram>,
    registry: Registry,
}

impl Metrics {
    pub fn new() -> Self {
        let messages_in = Family::default();
        let messages_out = Family::default();
        let messages_dropped = Family::default();
        let errors = Family::default();
        let process_duration = Family::<RouteLabels, Histogram>::new_with_constructor(|| {
            Histogram::new(exponential_buckets(0.0001, 2.0, 16))
        });

        let mut registry = Registry::default();
        registry.register("naps_messages_in", "Messages received", messages_in.clone());
        registry.register(
            "naps_messages_out",
            "Messages published",
            messages_out.clone(),
        );
        registry.register(
            "naps_messages_dropped",
            "Messages for which the processor returned no output",
            messages_dropped.clone(),
        );
        registry.register("naps_errors", "Errors by stage", errors.clone());
        registry.register(
            "naps_process_duration_seconds",
            "Processor duration",
            process_duration.clone(),
        );

        Self {
            messages_in,
            messages_out,
            messages_dropped,
            errors,
            process_duration,
            registry,
        }
    }

    pub fn encode(&self) -> String {
        let mut out = String::new();
        encode(&mut out, &self.registry).expect("encode metrics into a String");
        out
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}
