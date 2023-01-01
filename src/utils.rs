use std::collections::HashMap;
use anyhow::anyhow;
use crate::config::Config;
use poem::{Error, Result};
use reqwest::{StatusCode};
use poem_openapi::{Object};
use serde::Deserialize;
use tracing::error;

#[derive(Object, Debug)]
pub struct SolarResponse {
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


#[derive(Deserialize, Debug)]
struct SecondaryMeter {
    /// power produced by old pv system; data in watts
    #[serde(alias = "P")]
    power: f64,
}

#[derive(Deserialize, Debug)]
struct Inverter {
    /// power produced by both pv systems; data in watts
    #[serde(alias = "P")]
    cumulative_power: f64,
    /// current charge of the battery; data in percent
    #[serde(alias = "SOC")]
    battery_percent: f64,
}

#[derive(Deserialize, Debug)]
struct Site {
    /// how much power is drained from battery; negative value means the battery is charging; data in watts
    #[serde(alias = "P_Akku")]
    power_battery: f64,
    /// how much power is drained from grid; negative value means power is fed into the grid; data in watts
    #[serde(alias = "P_Grid")]
    power_grid: f64,
    /// how much power the whole house is consuming; always negative!; data in watts
    #[serde(alias = "P_Load")]
    house_consumption: f64,
    /// power produced by new pv system; data in watts
    #[serde(alias = "P_PV")]
    power_pv: f64,
    /// current autonomy of the system; data in percent
    #[serde(alias = "rel_Autonomy")]
    autonomy: f64,
    /// current self consumption value; data in percent
    #[serde(alias = "rel_SelfConsumption")]
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

async fn get_data(config: &Config) -> anyhow::Result<SolarResponse> {
    let client = reqwest::Client::new();
    let resp = client
        .get(config.inverter_url.clone().unwrap().join("/status/powerflow")?)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(anyhow!(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR)));
    }
    let text = resp.text().await?;
    let json: SolarJson = serde_json::from_str(text.as_str())?;

    let secondary_value = json.secondary_meters.values().last().unwrap_or_else(|| &SecondaryMeter {
        power: 0.0,
    });
    let inverter = json.inverters.last().unwrap_or_else(|| &Inverter {
        cumulative_power: 0.0,
        battery_percent: 0.0,
    });
    Ok(SolarResponse {
        old_inverter_power: secondary_value.power as u32,
        new_inverter_power: json.site.power_pv as u32,
        both_inverter_power: inverter.cumulative_power as u32,
        battery_load_percentage: inverter.battery_percent as u8,
        autonomy_percent: json.site.autonomy as u8,
        self_consumption_percent: json.site.self_consumption as u8,
        drain_from_battery: json.site.power_battery as i64,
        drain_from_grid: json.site.power_grid as i64,
        house_consumption: (-1.0 * json.site.house_consumption) as u64,
    })
}


pub async fn get_solar_values(config: &Config) -> Result<SolarResponse> {
    return match get_data(config).await {
        Ok(v) => { Ok(v) }
        Err(err) => {
            error!("{:?}", err);
            return Err(Error::from_status(StatusCode::INTERNAL_SERVER_ERROR));
        }
    };
}
