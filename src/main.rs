#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]
#![warn(clippy::dbg_macro, clippy::use_debug)]
#![warn(
clippy::unwrap_used,
clippy::expect_used,
clippy::panic,
clippy::unimplemented,
clippy::todo,
clippy::unreachable
)]
#![warn(
clippy::shadow_unrelated,
clippy::str_to_string,
clippy::wildcard_enum_match_arm
)]

extern crate core;

use std::{env, io};
use std::env::consts::ARCH;
use std::io::BufRead;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{ensure, Result};
use poem::{EndpointExt, Route, Server};
use poem::listener::TcpListener;
use poem::middleware::Cors;
use poem_openapi::OpenApiService;
use time::OffsetDateTime;
use tokio::spawn;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info, warn};

use crate::api::SolarApi;
use crate::config::{Config, load};
use crate::inverter::SolarData;
use crate::utils::add_point;
use crate::wattpilot::{Wattpilot, WattpilotData};

mod config;
mod utils;
mod api;
mod wattpilot;
mod inverter;

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    solar_data: Arc<RwLock<SolarData>>,
    wattpilot_data: Arc<RwLock<WattpilotData>>
}

#[tokio::main]
async fn start() -> Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::fmt::init();

    // check config values
    let mut config = load()?;
    ensure!(
        config.influx_url.is_some(),
        "Influx url should be set!"
    );
    ensure!(
        config.influx_measurement.is_some(),
        "Influx measurement should be set!"
    );
    ensure!(
         config.influx_token.is_some(),
        "Influx token should be set!"
    );
    ensure!(
        config.inverter_url.is_some(),
        "Inverter url should be set!"
    );
    ensure!(
        config.healthcheck_url.is_some(),
        "Healthchecks url should be set!"
    );
    if config.wattpilot_password.is_none() {
        println!("Wattpilot Passwort? ");
        let stdin = io::stdin();
        if let Some(Ok(line)) = stdin.lock().lines().next() {
            if line.is_empty() {
                warn!("No Wattpilot password given, feature deactivated!");
            } else {
                config.wattpilot_password = Some(line);
            }
        } else {
            warn!("No Wattpilot password given, feature deactivated!");
        }
    }

    let solar_data = Arc::new(RwLock::new(SolarData::default()));
    let wattpilot = Wattpilot::new(&config);
    let wp_clone;
    let wp_data_clone = match wattpilot {
        None => {
            wp_clone = None;
            Arc::default()
        }
        Some(wp) => {
            wp_clone = Some(Arc::clone(&wp));
            Arc::clone(&wp.read().await.data)
        }
    };

    // setup querying of Fronius and adding of data to db
    let config_clone = config.clone();
    let solar_data_clone = solar_data.clone();
    spawn(async move {
        loop {
            let now = OffsetDateTime::now_utc();
            let wait = u16::from(9 - now.second() % 10) * 1000 + 1000 - now.millisecond() % 1000;
            sleep(Duration::from_millis(u64::from(wait))).await;
            add_point(&config_clone, &solar_data_clone, &wp_clone).await;
        }
    });

    let server_url = format!("{}:{}", config.app_host.clone(), config.app_port.clone());
    let origins = config.allowed_origins.clone();

    // create api service and needed routes
    let mut api_service = OpenApiService::new(
        SolarApi,
        "HomeserverApi",
        env!("CARGO_PKG_VERSION"),
    );
    for server in config.swagger_servers.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        api_service = api_service.server(server);
    }
    // create var to carry db connection
    let state = AppState { config: Arc::new(config), solar_data, wattpilot_data: wp_data_clone};
    let ui = api_service.swagger_ui();
    let spec = api_service.spec();
    let api_route = Route::new()
        .nest_no_strip("/api", api_service)
        .data(state);
    let ui_route = Route::new().at("/", ui);

    // create routes for all things
    let route = Route::new()
        .nest_no_strip("/api", api_route)
        .at("/spec", poem::endpoint::make_sync(move |_| spec.clone()))
        .at("/", ui_route)
        .with(Cors::new().allow_origins(origins.split(',').map(str::trim).filter(|s| !s.is_empty())));

    // run server
    info!("Starting server at http://{}", server_url);
    Server::new(TcpListener::bind(server_url))
        .run(route)
        .await?;
    Ok(())
}

pub fn main() {
    let result = start();

    if let Err(err) = result {
        error!("Error: {}", err);
    }
}

