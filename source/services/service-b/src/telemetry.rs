use init_tracing_opentelemetry::config::Guard;

pub fn init() -> anyhow::Result<Guard> {
    let guard = init_tracing_opentelemetry::TracingConfig::production()
        .init_subscriber()?;
    Ok(guard)
}
