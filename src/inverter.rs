use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use anyhow::{anyhow};
use crate::config::Config;
use poem::{Error};
use reqwest::{StatusCode};
use poem_openapi::{Object};
use serde::{Deserialize};
use time::OffsetDateTime;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tracing::{error, info};
use crate::utils::deserialize_null_default;

#[derive(Object, Debug, Clone)]
pub struct SolarData {
    /// last time fronius was queried
    pub(crate) last_time: OffsetDateTime,
    /// power produced by old pv system; data in watts
    pub(crate) old_inverter_power: u32,
    /// power produced by new pv system; data in watts
    pub(crate) new_inverter_power: u32,
    /// power produced by both pv systems; data in watts
    pub(crate) both_inverter_power: u32,
    /// current charge of the battery; data in percent
    pub(crate) battery_load_percentage: u8,
    /// current autonomy of the system; data in percent
    pub(crate) autonomy_percent: u8,
    /// current self consumption value; data in percent
    pub(crate) self_consumption_percent: u8,
    /// how much power is drained from battery; negative value means the battery is charging; data in watts
    pub(crate) drain_from_battery: i64,
    /// how much power is drained from grid; negative value means power is fed into the grid; data in watts
    pub(crate) drain_from_grid: i64,
    /// how much power the whole house is consuming; data in watts
    pub(crate) house_consumption: u64,
}

impl Default for SolarData {
    fn default() -> Self {
        SolarData {
            last_time: OffsetDateTime::UNIX_EPOCH,
            old_inverter_power: Default::default(),
            new_inverter_power: Default::default(),
            both_inverter_power: Default::default(),
            battery_load_percentage: Default::default(),
            autonomy_percent: Default::default(),
            self_consumption_percent: Default::default(),
            drain_from_battery: Default::default(),
            drain_from_grid: Default::default(),
            house_consumption: Default::default(),
        }
    }
}


#[derive(Deserialize, Debug)]
struct SecondaryMeter {
    /// power produced by old pv system; data in watts
    #[serde(alias = "P", deserialize_with = "deserialize_null_default")]
    power: f64,
}

#[derive(Deserialize, Debug)]
struct Inverter {
    /// current charge of the battery; data in percent
    #[serde(alias = "SOC", default, deserialize_with = "deserialize_null_default")]
    battery_percent: f64,
}

#[derive(Deserialize, Debug)]
struct Site {
    /// how much power is drained from battery; negative value means the battery is charging; data in watts
    #[serde(alias = "P_Akku", deserialize_with = "deserialize_null_default")]
    power_battery: f64,
    /// how much power is drained from grid; negative value means power is fed into the grid; data in watts
    #[serde(alias = "P_Grid", deserialize_with = "deserialize_null_default")]
    power_grid: f64,
    /// how much power the whole house is consuming; always negative!; data in watts
    #[serde(alias = "P_Load", deserialize_with = "deserialize_null_default")]
    house_consumption: f64,
    /// power produced by new pv system; data in watts
    #[serde(alias = "P_PV", deserialize_with = "deserialize_null_default")]
    power_pv: f64,
    /// current autonomy of the system; data in percent
    #[serde(alias = "rel_Autonomy", deserialize_with = "deserialize_null_default")]
    autonomy: f64,
    /// current self consumption value; data in percent
    #[serde(alias = "rel_SelfConsumption", deserialize_with = "deserialize_null_default")]
    self_consumption: f64,
}

#[derive(Deserialize, Debug)]
struct SolarJson {
    #[serde(alias = "SecondaryMeters")]
    secondary_meters: HashMap<String, SecondaryMeter>,
    #[serde(alias = "inverters")]
    inverters: Vec<Inverter>,
    #[serde(alias = "inverters")]
    site: Site,
}

async fn get_data(config: &Config) -> anyhow::Result<SolarData> {
    let mut json_opt: Option<SolarJson> = None;
    let mut error: Option<String> = None;
    let mut success: bool = false;
    let sleep_time = Duration::from_millis(100);
    for _ in 0..3 {
        let client = reqwest::Client::new();
        let resp = client
            .get(config.inverter_url.clone().unwrap().join("/status/powerflow")?)
            .send()
            .await?;
        if !resp.status().is_success() {
            error = Some(format!("Response Error: {}, {}", resp.status(), resp.text().await?));
            sleep(sleep_time).await;
            continue;
        }
        let text = resp.text().await?;
        match serde_json::from_str::<SolarJson>(text.as_str()) {
            Ok(v) => {
                success = true;
                json_opt = Some(v);
                break;
            }
            Err(err) => {
                error = Some(format!("Json Error: {}, {}", err, text));
                sleep(sleep_time).await;
                continue;
            }
        };
    }
    if !success {
        error!("{}", error.unwrap_or_default());
        return Err(anyhow!(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)));
    }

    let Some(json) = json_opt else {
        error!("json_opt is empty");
        return Err(anyhow!(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)));
    };
    let secondary_value = json.secondary_meters.values().last().unwrap_or(&SecondaryMeter {
        power: 0.0,
    });
    let inverter = json.inverters.last().unwrap_or(&Inverter {
        battery_percent: 0.0,
    });
    Ok(SolarData {
        last_time: OffsetDateTime::now_utc(),
        old_inverter_power: secondary_value.power as u32,
        new_inverter_power: json.site.power_pv as u32,
        both_inverter_power: (secondary_value.power + json.site.power_pv) as u32,
        battery_load_percentage: inverter.battery_percent as u8,
        autonomy_percent: json.site.autonomy as u8,
        self_consumption_percent: json.site.self_consumption as u8,
        drain_from_battery: json.site.power_battery as i64,
        drain_from_grid: json.site.power_grid as i64,
        house_consumption: (-1.0 * json.site.house_consumption) as u64,
    })
}


pub(crate) async fn fetch_solar_values(config: &Config, solar_data: Arc<RwLock<SolarData>>) -> bool {
    info!("Fetching data from Fronius at {}", OffsetDateTime::now_utc());
    match get_data(config).await {
        Ok(v) => {
            *solar_data.write().await = v;
            true
        }
        Err(err) => {
            error!("{:?}", err);
            false
        }
    }
}