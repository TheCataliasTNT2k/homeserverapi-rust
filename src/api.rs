use crate::AppState;
use poem::web::{Data};
use poem::{Result};
use poem_openapi::{ApiResponse, OpenApi, Tags};
use poem_openapi::payload::Json;
use crate::utils::{get_solar_values, SolarResponse};

// GLOBALS -----------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------------

// ERRORS ------------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------------------

// RESPONSES ---------------------------------------------------------------------------------------

#[derive(ApiResponse)]
enum SolarResp {
    /// everything is fine
    #[oai(status = 200)]
    Ok(Json<SolarResponse>),

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
    /// get current inverter values
    #[oai(path = "/", method = "get")]
    async fn get_values(
        &self,
        state: Data<&AppState>,
    ) -> Result<SolarResp> {
        let data = get_solar_values(&state.config).await?;
        Ok(SolarResp::Ok(Json(data)))
    }
}
