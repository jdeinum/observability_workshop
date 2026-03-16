use prometheus_client::encoding::EncodeLabelSet;
use prometheus_client::metrics::exemplar::HistogramWithExemplars;
use prometheus_client::metrics::family::{Family, MetricConstructor};
use prometheus_client::registry::Registry;

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct HttpLabels {
    pub method: String,
    pub path: String,
    pub status: String,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq, EncodeLabelSet)]
pub struct TraceExemplar {
    pub trace_id: String,
}

const HTTP_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

#[derive(Clone)]
pub struct HttpDurationConstructor;

impl MetricConstructor<HistogramWithExemplars<TraceExemplar>> for HttpDurationConstructor {
    fn new_metric(&self) -> HistogramWithExemplars<TraceExemplar> {
        HistogramWithExemplars::new(HTTP_DURATION_BUCKETS.iter().copied())
    }
}

pub struct Metrics {
    pub http_duration:
        Family<HttpLabels, HistogramWithExemplars<TraceExemplar>, HttpDurationConstructor>,
}

impl Metrics {
    pub fn new(registry: &mut Registry) -> Self {
        let http_duration = Family::new_with_constructor(HttpDurationConstructor);

        registry.register(
            "http_request_duration_seconds",
            "HTTP request duration in seconds",
            http_duration.clone(),
        );

        Self { http_duration }
    }
}
