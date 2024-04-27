use std::env;
use std::sync::Arc;
use poem::http::header::AUTHORIZATION;
use serde::{Deserialize, Deserializer};
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tracing::{error, info};
use tracing::log::warn;
use crate::config::Config;
use crate::inverter::{fetch_solar_values, SolarData};
use crate::wattpilot::{Wattpilot, WattpilotData};

pub(crate) fn deserialize_null_default<'de, D, T>(deserializer: D) -> poem::Result<T, D::Error>
    where
        T: Default + Deserialize<'de>,
        D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

async fn contact_monitoring(config: &Config, code: u32, body: Option<String>) {
    let client2 = reqwest::Client::new();

    // config will have this field checked at this time
    #[allow(clippy::unwrap_used)]
        let mut url = config.healthcheck_url.clone().unwrap();
    #[allow(clippy::unwrap_used)]
    url.path_segments_mut().unwrap().push(code.to_string().as_str());
    if let Err(err) = match body {
        None => {
            client2
                .post(url)
                .send()
                .await
        }
        Some(b) => {
            client2
                .post(url)
                .body(b)
                .send()
                .await
        }
    } {
        error!("Error while contacting monitoring: {err}");
    }
}

/// add point to database
pub(crate) async fn add_point(
    config: &Config,
    solar_data: &Arc<RwLock<SolarData>>,
    wp_arc: &Option<Arc<RwLock<Wattpilot>>>
) {
    let actual_time = OffsetDateTime::now_utc();
    if !fetch_solar_values(config, solar_data.clone()).await {
        contact_monitoring(config, 1, Some("Solar values could not be fetched".to_owned())).await;
        return;
    }
    if env::var("NO_DB").is_ok() {
        contact_monitoring(config, 0, None).await;
        return;
    }
    info!("Adding point to database {}", actual_time);
    let solar = solar_data.read().await;
    let solar_age = (OffsetDateTime::now_utc() - solar.last_time).as_seconds_f64();
    if solar_age > 30f64 {
        warn!("Solar data too old: {solar_age}");
        contact_monitoring(config, 2, Some(format!("Solar data too old: {solar_age}").to_owned())).await;
    }
    let wp = match wp_arc {
        None => WattpilotData::default(),
        Some(some) => {
            let read = some.read().await;
            let wp_age = (OffsetDateTime::now_utc() - read.data.read().await.last_updated).as_seconds_f64();
            if !read.authenticated || wp_age > 30f64  {
                warn!("Wattpilot data too old: {wp_age}");
                contact_monitoring(config, 2, Some(format!("Wattpilot data too old: {wp_age}").to_owned())).await;
                WattpilotData::default()
            } else {
                read.data.read().await.clone()
            }
        }
    };
    // has been checked before
    #[allow(clippy::unwrap_used)]
    let body = format!(
        "{} old={},new={},both={},battery_percentage={},autonomy_percentage={},self_consumption_percentage={},drain_from_battery={},drain_from_grid={},house_consumption={},\
        wp_charging_values=\"{}\",wp_car_state={},wp_model_status={},wp_wh={},wp_tpcm={},wp_lps={},wp_ets={},wp_power={} \
        {}",
        config.influx_measurement.clone().unwrap(),
        // solar stuff
        solar.old_inverter_power,
        solar.new_inverter_power,
        solar.both_inverter_power,
        solar.battery_load_percentage,
        solar.autonomy_percent,
        solar.self_consumption_percent,
        solar.drain_from_battery,
        solar.drain_from_grid,
        solar.house_consumption,
        // wp stuff
        serde_json::to_string(&wp.charging_values).unwrap(),
        serde_json::to_string(&wp.car_state).unwrap(),
        serde_json::to_string(&wp.model_status).unwrap(),
        wp.charged_since_connected,
        serde_json::to_string(&wp.tpcm).unwrap(),
        wp.lps,
        wp.ets,
        wp.charging_values.pt,
        // timestamp
        actual_time.unix_timestamp()
    );
    let client = reqwest::Client::new();
    // unwraps can not panic
    #[allow(clippy::unwrap_used)]
    match client
        .post(format!("{}&precision=s", config.influx_url.clone().unwrap()))
        .header(AUTHORIZATION, format!("Token {}", config.influx_token.clone().unwrap()))
        .body(body)
        .send()
        .await {
        Ok(v) => {
            if v.status().is_success() {
                contact_monitoring(config, 0, None).await;
            } else {
                error!("Influx success Error: {:?}", v);
                if let Ok(text) = v.text().await {
                    error!("Influx success Error: {:?}", text);
                }
                contact_monitoring(config, 2, Some("Failed to put data into influx".to_owned())).await;
            }
        }
        Err(err) => {
            error!("Influx response Error: {}", err);
            contact_monitoring(config, 2, Some("Failed to put data into influx".to_owned())).await;
        }
    };
}
