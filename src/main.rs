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

mod config;
mod utils;
mod api;

use std::env;
use std::sync::Arc;
use std::time::Duration;
use crate::config::{Config, load};
use tracing::{error, info, warn};
use anyhow::{ensure, Result};
use poem::{EndpointExt, Route, Server};
use poem::listener::TcpListener;
use poem::middleware::Cors;
use poem_openapi::OpenApiService;
use reqwest::header::AUTHORIZATION;
use time::OffsetDateTime;
use tokio::spawn;
use tokio::time::{sleep};
use crate::api::SolarApi;
use crate::utils::get_solar_values;


// INFLUX_TOKEN=_Wx-FNyW-83tNkg9dEsWTKR0j-qXMhKgOgP-WujpLTW_JLmJalUIyeyuRFwDknjHm6WHgN73StZ0VVnX6JMViA==;
// INFLUX_URL=http://influx.server.home/api/v2/write?org=strom&bucket=test;


/// add point to database
async fn add_point(config: &Config) {
    let actual_time = OffsetDateTime::now_utc();
    info!("Adding point to database {}", actual_time);
    let data = match get_solar_values(&config).await {
        Ok(v) => { v }
        Err(err) => {
            error!("{:?}", err);
            return;
        }
    };
    let body = format!(
        "solar old={},new={},both={},battery_percentage={},autonomy_percentage={},self_consumption_percentage={},drain_from_battery={},drain_from_grid={},house_consumption={} {}",
        data.old_inverter_power,
        data.new_inverter_power,
        data.both_inverter_power,
        data.battery_load_percentage,
        data.autonomy_percent,
        data.self_consumption_percent,
        data.drain_from_battery,
        data.drain_from_grid,
        data.house_consumption,
        actual_time.unix_timestamp()
    );
    let client = reqwest::Client::new();
    let success;
    match client
        .post(format!("{}&precision=s", config.influx_url.clone().unwrap()))
        .header(AUTHORIZATION, format!("Token {}", config.influx_token.clone().unwrap()))
        .body(body)
        .send()
        .await {
        Ok(v) => {
            success = v.status().is_success();
            if !success {
                error!("{:?}", v);
            }
        }
        Err(err) => {
            error!("{}", err);
            success = false;
        }
    };
    let fail = match success {
        true => {0}
        false => { 1 }
    };
    let client = reqwest::Client::new();

    let mut url = config.healthcheck_url.clone().unwrap();
    url.path_segments_mut().unwrap().push(fail.to_string().as_str());
    let _ = client
        .get(url)
        .send()
        .await;
}

#[derive(Debug, Clone)]
struct AppState {
    config: Arc<Config>,
}

#[tokio::main]
async fn start() -> Result<()> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "debug");
    }
    tracing_subscriber::fmt::init();

    // check config values
    let config = load()?;
    ensure!(
        config.influx_url.is_some(),
        "Influx url should be set!"
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

    // setup addition of points
    let config_clone = config.clone();
    spawn(async move {
        loop {
            let now = OffsetDateTime::now_utc();
            let wait = (29 - now.second() % 30) as u16 * 1000 + 1000 - now.millisecond() % 1000;
            sleep(Duration::from_millis(wait as u64)).await;
            add_point(&config_clone).await;
        }
    });

    let server_url = format!("{}:{}", config.app_host.clone(), config.app_port.clone());
    // create var to carry db connection
    let state = AppState { config: Arc::new(config) };

    // create api service and needed routes
    let api_service = OpenApiService::new(
        SolarApi,
        "HomeserverApi",
        env!("CARGO_PKG_VERSION"),
    )
        .server(format!("http://{server_url}"));
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
        .with(Cors::new());

    // run server
    info!("Starting server at http://{}", server_url);
    let x = Server::new(TcpListener::bind(server_url))
        .run(route)
        .await?;
    return Ok(x);
}

pub fn main() {
    let result = start();

    if let Err(err) = result {
        error!("Error: {}", err);
    }
}

