use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct Settings {
    pub application: ApplicationSettings,
    pub database: DatabaseSettings,
    pub redis_url: String,
    pub otlp_endpoint: String,
    pub pyroscope_url: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct ApplicationSettings {
    pub host: String,
    pub port: u16,
    pub metrics_port: u16,
    pub service_name: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct DatabaseSettings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub name: String,
    pub pool_max_connections: u32,
    pub pool_min_connections: u32,
    pub pool_acquire_timeout_secs: u64,
}

impl DatabaseSettings {
    pub fn connection_url(&self) -> String {
        format!(
            "postgres://{}:{}@{}:{}/{}",
            self.username, self.password, self.host, self.port, self.name
        )
    }
}

impl Settings {
    pub fn new() -> Result<Self, config::ConfigError> {
        let run_mode = std::env::var("RUN_MODE").unwrap_or_else(|_| "local".into());

        let settings = config::Config::builder()
            .add_source(config::File::with_name("configuration/base"))
            .add_source(config::File::with_name("configuration/service-c"))
            .add_source(config::File::with_name(&format!("configuration/{}", run_mode)).required(false))
            .add_source(config::Environment::with_prefix("APP").separator("__"))
            .build()?;

        settings.try_deserialize()
    }
}
