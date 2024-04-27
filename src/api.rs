use poem::Result;
use poem::web::Data;
use poem_openapi::{ApiResponse, Object, OpenApi, Tags};
use poem_openapi::payload::Json;

use crate::AppState;
use crate::inverter::SolarData;
use crate::wattpilot::WattpilotData;

// GLOBALS -----------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------------

// OBJECTS -----------------------------------------------------------------------------------------
#[derive(Object)]
struct SolarRespData {
    /// data of the connected wattpilot
    wattpilot_data: WattpilotData,
    /// data of rest of system
    solar_data: SolarData,
}
// -------------------------------------------------------------------------------------------------

// ERRORS ------------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------------

// RESPONSES ---------------------------------------------------------------------------------------

#[derive(ApiResponse)]
enum SolarResp {
    /// everything is fine
    #[oai(status = 200)]
    Ok(Json<SolarRespData>),

    /// something went wrong
    #[oai(status = 500)]
    #[allow(dead_code)]
    InternalServerError,
}
// -------------------------------------------------------------------------------------------------

// REQUESTS ----------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------------


pub(crate) struct SolarApi;

#[derive(Tags)]
enum Tag {
    Solar,
}

#[OpenApi(prefix_path = "/api/solar", tag = "Tag::Solar")]
impl SolarApi {
    /// get current system values
    #[oai(path = "/", method = "get")]
    async fn get_values(
        &self,
        state: Data<&AppState>,
    ) -> Result<SolarResp> {
        Ok(
            SolarResp::Ok(
                Json(
                    SolarRespData {
                        wattpilot_data: state.wattpilot_data.read().await.clone(),
                        solar_data: state.solar_data.read().await.clone(),
                    }
                )
            )
        )
    }
}
