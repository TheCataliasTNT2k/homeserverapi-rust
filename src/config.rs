//! Global configuration from environment variables

use anyhow::Result;
use serde::Deserialize;
use url::Url;

/// Values from environment variables
#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// url for healthchecks host.domain:port/xy
    pub healthcheck_url: Option<Url>,

    /// connect uri for database host.domain:port/xy
    pub influx_url: Option<Url>,

    /// token for influx database
    pub influx_token: Option<String>,

    /// url for the inverter
    pub inverter_url: Option<Url>,

    /// ip to bind the http server
    pub app_host: String,

    /// port to bind the http server
    pub app_port: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            healthcheck_url: None,
            influx_url: None,
            influx_token: None,
            inverter_url: None,
            app_host: "127.0.0.1".to_owned(),
            app_port: "3000".to_owned(),
        }
    }
}

/// load configuration from environment variables
pub fn load() -> Result<Config> {
    Ok(config::Config::builder()
        .add_source(config::Environment::default())
        .build()?
        .try_deserialize()?)
}
