use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub redis_url: String,
    pub otlp_endpoint: String,
    pub pyroscope_url: String,
    pub cache: CacheSettings,
    pub services: ServiceUrls,
}

#[derive(Deserialize, Clone, Debug)]
pub struct CacheSettings {
    pub ttl_secs: u64,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ServiceUrls {
    pub service_b_url: String,
    pub service_d_url: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ApplicationSettings {
    pub host: String,
    pub port: u16,
    pub metrics_port: u16,
    pub service_name: String,
}

impl Settings {
    pub fn new() -> Result<Self, config::ConfigError> {
        let run_mode = std::env::var("RUN_MODE").unwrap_or_else(|_| "local".into());

        let settings = config::Config::builder()
            .add_source(config::File::with_name("configuration/base"))
            .add_source(config::File::with_name("configuration/service-a"))
            .add_source(config::File::with_name(&format!("configuration/{}", run_mode)).required(false))
            .add_source(config::Environment::with_prefix("APP").separator("__"))
            .build()?;

        settings.try_deserialize()
    }
}
