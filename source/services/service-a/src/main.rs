use std::sync::Arc;

use axum::http::header;
use axum::response::IntoResponse;
use prometheus_client::encoding::text::encode;
use prometheus_client::registry::Registry;
use pyroscope::PyroscopeAgent;
use pyroscope_pprofrs::{pprof_backend, PprofConfig};
use service_a_gateway::{app, metrics::Metrics, settings, telemetry};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load settings
    let settings = settings::Settings::new()?;

    // Initialize telemetry
    let _guard = telemetry::init()?;

    // Initialize Pyroscope profiling
    let _pyroscope_agent = PyroscopeAgent::builder(&settings.pyroscope_url, &settings.application.service_name)
        .backend(pprof_backend(PprofConfig::new().sample_rate(100)))
        .build()
        .expect("failed to build pyroscope agent")
        .start()
        .expect("failed to start pyroscope agent");

    tracing::info!(
        service = %settings.application.service_name,
        port = %settings.application.port,
        "Starting service"
    );

    // Initialize Prometheus metrics registry
    let mut registry = Registry::default();
    let metrics = Arc::new(Metrics::new(&mut registry));
    let registry = Arc::new(registry);

    // Build application
    let app = app::build(settings.clone(), Arc::clone(&metrics)).await?;

    // Start metrics server
    let metrics_addr = format!("{}:{}", settings.application.host, settings.application.metrics_port);
    tokio::spawn(async move {
        let metrics_app = axum::Router::new()
            .route("/metrics", axum::routing::get(move || {
                let registry = Arc::clone(&registry);
                async move {
                    let mut buf = String::new();
                    encode(&mut buf, &registry).expect("encoding metrics failed");
                    (
                        [(header::CONTENT_TYPE, "application/openmetrics-text; version=1.0.0; charset=utf-8")],
                        buf,
                    ).into_response()
                }
            }));

        let listener = tokio::net::TcpListener::bind(&metrics_addr)
            .await
            .expect("failed to bind metrics port");

        tracing::info!("Metrics server listening on {}", metrics_addr);

        axum::serve(listener, metrics_app)
            .await
            .expect("metrics server failed");
    });

    // Start main application server
    let addr = format!("{}:{}", settings.application.host, settings.application.port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("Application listening on {}", addr);

    axum::serve(listener, app)
        .await?;

    Ok(())
}
