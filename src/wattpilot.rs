use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use futures_util::{SinkExt, StreamExt};
use futures_util::stream::{SplitSink, SplitStream};
use pbkdf2::pbkdf2_hmac_array;
use poem_openapi::{Enum, Object};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serde_repr::{Deserialize_repr, Serialize_repr};
use sha2::{Digest, Sha256, Sha512};
use time::OffsetDateTime;
use tokio::net::TcpStream;
use tokio::spawn;
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};
use url::Url;

use crate::config::Config;

#[derive(Deserialize, Serialize, Debug)]
struct HelloMessage {
    serial: String,
    secured: bool,
}

#[derive(Deserialize, Serialize, Debug)]
struct AuthRequiredMessage {
    token1: String,
    token2: String,
}


#[derive(Debug, Clone, Object)]
pub(crate) struct WattpilotData {
    /// timestamp of last received update
    pub last_updated: OffsetDateTime,
    /// current rate of charge
    pub charging_values: ChargingValues,
    /// state of car
    pub car_state: CarState,
    /// ?
    pub model_status: ModelStatus,
    /// how many Wh were put into the car since was connected
    pub charged_since_connected: f64,
    /// ?
    pub tpcm: String,
    /// ?
    pub lps: i64,
    /// ?
    pub ets: i64
}

impl Default for WattpilotData {
    fn default() -> Self {
        WattpilotData {
            last_updated: OffsetDateTime::UNIX_EPOCH,
            charging_values: Default::default(),
            car_state: CarState::Unknown,
            model_status: ModelStatus::NotChargingBecauseNoChargeCtrlData,
            charged_since_connected: 0f64,
            tpcm: "[]".to_owned(),
            lps: 0,
            ets: 0
        }
    }
}

#[derive(Debug, Clone, Object, Default, Serialize, Deserialize)]
pub(crate) struct ChargingValues {
    // U (L1, L2, L3, N), I (L1, L2, L3),        P (L1, L2, L3, N, Total), pf (L1, L2, L3, N)
    pub(crate) u1: f32,
    pub(crate) u2: f32,
    pub(crate) u3: f32,
    pub(crate) un: f32,
    pub(crate) i1: f32,
    pub(crate) i2: f32,
    pub(crate) i3: f32,
    pub(crate) p1: f32,
    pub(crate) p2: f32,
    pub(crate) p3: f32,
    pub(crate) pn: f32,
    pub(crate) pt: f32,
    pub(crate) pf1: f32,
    pub(crate) pf2: f32,
    pub(crate) pf3: f32,
    pub(crate) pfn: f32,
}

#[derive(Deserialize_repr, Serialize_repr, Clone, Debug, Enum)]
#[repr(u16)]
pub(crate) enum CarState {
    Unknown = 0,
    Idle = 1,
    Charging = 2,
    WaitCar = 3,
    Complete = 4,
    Error = 5,
}

#[derive(Serialize_repr, Deserialize_repr, Clone, Debug, Enum)]
#[repr(u16)]
pub(crate) enum ModelStatus {
    NotChargingBecauseNoChargeCtrlData = 0,
    NotChargingBecauseOverTemperature = 1,
    NotChargingBecauseAccessControlWait = 2,
    ChargingBecauseForceStateOn = 3,
    NotChargingBecauseForceStateOff = 4,
    NotChargingBecauseScheduler = 5,
    NotChargingBecauseEnergyLimit = 6,
    ChargingBecauseAwattarPriceLow = 7,
    ChargingBecauseAutomaticStopTestLadung = 8,
    ChargingBecauseAutomaticStopNotEnoughTime = 9,
    ChargingBecauseAutomaticStop = 10,
    ChargingBecauseAutomaticStopNoClock = 11,
    ChargingBecausePvSurplus = 12,
    ChargingBecauseFallbackGoEDefault = 13,
    ChargingBecauseFallbackGoEScheduler = 14,
    ChargingBecauseFallbackDefault = 15,
    NotChargingBecauseFallbackGoEAwattar = 16,
    NotChargingBecauseFallbackAwattar = 17,
    NotChargingBecauseFallbackAutomaticStop = 18,
    ChargingBecauseCarCompatibilityKeepAlive = 19,
    ChargingBecauseChargePauseNotAllowed = 20,
    NotChargingBecauseSimulateUnplugging = 22,
    NotChargingBecausePhaseSwitch = 23,
    NotChargingBecauseMinPauseDuration = 24,
}


#[derive(Debug)]
pub(crate) struct Wattpilot {
    secured: bool,
    hashed_pw: String,
    url: Url,
    pub(crate) data: Arc<RwLock<WattpilotData>>,
    write: Arc<RwLock<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
    pub(crate) authenticated: bool
}

impl Wattpilot {
    pub(crate) fn new(config: &Config) -> Option<Arc<RwLock<Wattpilot>>> {
        if config.wattpilot_url.is_none() || config.wattpilot_password.is_none() {
            info!("Wattpilot url or wattpilot password is not set, wattpilot feature deactivated!");
            None
        } else {
            #[allow(clippy::unwrap_used)]
                let Ok(url) = config.wattpilot_url.clone().unwrap().join("ws") else {
                error!("Failed to concat url in wattpilot main loop");
                return None;
            };
            // can not panic, because we tested above, that both values exist!
            let wp = Arc::new(RwLock::new(Wattpilot {
                secured: false,
                hashed_pw: String::new(),
                url,
                authenticated: false,
                data: Arc::default(),
                write: Arc::default()
            }));
            let config_clone = config.clone();
            let wp_clone = Arc::clone(&wp);
            spawn(async {
                Wattpilot::main_handler(wp_clone, config_clone).await;
            });
            Some(wp)
        }
    }

    pub async fn send(&self, secure: bool, payload: String, message_id: &str) -> Result<()> {
        let message = if secure {
            let hmac = "";
            //h = hmac.new(bytearray(self._hashedpassword), bytearray(payload.encode()), hashlib.sha256)
            json!({
            "type": "securedMsg", "data": payload, "requestId": message_id.to_owned() + "sm", "hmac": hmac
        }).to_string()
        } else {
            payload
        };
        let Some(ref mut write) = &mut *self.write.write().await else {
            return Err(anyhow!("Could not send message because no websocket: {}", message));
        };
        if let Err(err) = write.send(Message::from(message)).await {
            return Err(anyhow!(err));
        };
        Ok(())
    }

    async fn authenticate(
        &mut self,
        password: String,
        read: &mut SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    ) -> Result<()> {
        let Some(x) = read.next().await else {
            return Err(anyhow!("No data for 'hello' message"));
        };
        let hello_message: HelloMessage = serde_json::from_str(x?.to_text()?)?;
        let Some(x) = read.next().await else {
            return Err(anyhow!("No data for 'auth' message"));
        };
        let auth_message: AuthRequiredMessage = serde_json::from_str(x?.to_text()?)?;

        if self.hashed_pw.is_empty() {
            let array = pbkdf2_hmac_array::<Sha512, 32>(password.as_ref(), hello_message.serial.as_ref(), 100_000);
            self.hashed_pw = BASE64_STANDARD.encode(array)[..32].to_owned();
        }

        let mut hasher1 = Sha256::new();
        hasher1.update(auth_message.token1 + &self.hashed_pw);

        let token3 = &format!("{:#032x}", rand::thread_rng().gen_range(u128::MAX / 2..u128::MAX))[2..34].to_owned();
        let mut hasher2 = Sha256::new();
        hasher2.update(token3.to_owned() + &auth_message.token2 + &format!("{:x}", hasher1.finalize()));

        self.send(
            false,
            json!({"type": "auth", "token3": token3, "hash": &format!("{:x}", hasher2.finalize())}).to_string(),
            "",
        ).await?;

        let Some(x) = read.next().await else {
            return Err(anyhow!("No data for 'auth' response"));
        };
        let v: Value = serde_json::from_str(x?.to_text()?)?;
        if v["type"] == "authError" {
            error!("Authentication failed! {}", v["message"]);
            self.authenticated = false;
        }
        if v["type"] == "authSuccess" {
            info!("Authentication succeeded!");
            self.authenticated = true;
        }
        Ok(())
    }

    pub(crate) async fn main_handler(wp: Arc<RwLock<Wattpilot>>, config: Config) {
        loop {
            info!("Trying to connect to wattpilot ...");
            let mut wp_write = wp.write().await;
            wp_write.authenticated = false;
            let (stream, _) = match connect_async(wp_write.url.clone()).await {
                Ok(x) => { x }
                Err(err) => {
                    error!("Error while connecting to wattpilot: {:#?}", err);
                    sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };
            info!("Wattpilot Websocket connected.");
            let (write, mut read) = stream.split();
            wp_write.write = Arc::new(RwLock::new(Some(write)));
            // can not be none, we tested before
            #[allow(clippy::unwrap_used)]
            if let Err(err) = wp_write.authenticate(config.wattpilot_password.clone().unwrap(), &mut read).await {
                error!("Websocket authentication failed!");
                error!("{:#?}", err);
                wp.write().await.authenticated = false;
                sleep(Duration::from_secs(3)).await;
                info!("Trying to connect to wattpilot again...");
                continue;
            }
            let data = Arc::clone(&wp_write.data);
            drop(wp_write);
            while let Some(message) = read.next().await {
                if let Ok(msg) = message {
                    if let Ok(text) = msg.to_text() {
                        Wattpilot::read_message(&data, text).await;
                    }
                } else {
                    error!("Error receiving message, restarting websocket");
                    wp.write().await.authenticated = false;
                }
            }
            sleep(Duration::from_secs(3)).await;
        }
    }

    #[allow(clippy::shadow_unrelated)]
    async fn read_message(data: &Arc<RwLock<WattpilotData>>, message: &str) {
        let Ok(v) = serde_json::from_str::<Value>(message) else {
            return;
        };
        let Some(status) = v.get("status") else {
            return;
        };
        let Some(obj) = status.as_object() else {
            return;
        };
        let reduced: HashMap<&str, &Value> = obj.iter().filter_map(|(key, value)| {
            if ["nrg", "car", "modelStatus", "wh", "tpcm", "lps", "ets"].contains(&&**key) {
                return Some((key.as_str(), value));
            }
            None
        }).collect();
        if reduced.is_empty() {
            return;
        }
        let mut lock = data.write().await;
        lock.last_updated = OffsetDateTime::now_utc();
        if let Some(data) = reduced.get("nrg") {
            if let Ok(parsed_value) = serde_json::from_value::<ChargingValues>((*data).clone()) {
                lock.charging_values = parsed_value;
            } else {
                warn!("Could not parse as nrg: {}", data);
            }
        }
        if let Some(data) = reduced.get("car") {
            if let Ok(parsed_value) = serde_json::from_value::<CarState>((*data).clone()) {
                lock.car_state = parsed_value;
            } else {
                warn!("Could not parse as car: {}", data);
            }
        }
        if let Some(data) = reduced.get("modelStatus") {
            if let Ok(parsed_value) = serde_json::from_value::<ModelStatus>((*data).clone()) {
                lock.model_status = parsed_value;
            } else {
                warn!("Could not parse as modelStatus: {}", data);
            }
        }
        if let Some(data) = reduced.get("wh") {
            if let Ok(parsed_value) = serde_json::from_value::<f64>((*data).clone()) {
                lock.charged_since_connected = parsed_value;
            } else {
                warn!("Could not parse as wh: {}", data);
            }
        }
        if let Some(data) = reduced.get("tpcm") {
            #[allow(clippy::wildcard_enum_match_arm)]
            match data {
                Value::Array(array) => {
                    if let Ok(parsed_value) = serde_json::to_string(array) {
                        lock.tpcm = parsed_value;
                    } else {
                        warn!("Could not parse as tpcm: {}", data);
                    }
                }
                _ => {
                    warn!("Could not parse as tpcm: {}", data);
                }
            }

        }
        if let Some(data) = reduced.get("lps") {
            if let Ok(parsed_value) = serde_json::from_value::<i64>((*data).clone()) {
                lock.lps = parsed_value;
            } else {
                warn!("Could not parse as lps: {}", data);
            }
        }
        if let Some(data) = reduced.get("ets") {
            if let Ok(parsed_value) = serde_json::from_value::<i64>((*data).clone()) {
                lock.ets = parsed_value;
            } else {
                warn!("Could not parse as ets: {}", data);
            }
        }
    }
}

