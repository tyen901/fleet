use fleet_app::services::FleetServices;
use tauri::State;

// Bridge wrapper for services
#[allow(dead_code)]
pub struct AppState {
    pub services: FleetServices,
}

impl AppState {
    pub fn new(services: FleetServices) -> Self {
        Self { services }
    }
}

// Error adapter for Tauri
#[derive(Debug, serde::Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

impl From<fleet_app::AppError> for ApiError {
    fn from(e: fleet_app::AppError) -> Self {
        ApiError {
            code: "app_error",
            message: e.to_string(),
        }
    }
}

// Example command stub
#[tauri::command]
pub async fn get_system_status(_state: State<'_, AppState>) -> Result<String, ApiError> {
    Ok("Ready".into())
}
