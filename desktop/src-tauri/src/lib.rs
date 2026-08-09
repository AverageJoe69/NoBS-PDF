use std::{path::PathBuf, sync::Mutex};

use pdfdoctor::app::{
    self, AppError, CancellationToken, DocumentSummary, OptimisationEstimate, OptimisationResult,
};
use tauri::{AppHandle, Emitter, Manager, State};

mod licensing;
use licensing::LicenceStatus;

struct OptimisationState(Mutex<CancellationToken>);

#[tauri::command]
async fn inspect_pdf(path: String) -> Result<DocumentSummary, AppError> {
    require_licence()?;
    tauri::async_runtime::spawn_blocking(move || app::inspect_pdf(&PathBuf::from(path)))
        .await
        .map_err(join_error)?
}

#[tauri::command]
async fn estimate_pdf(path: String, profile: String) -> Result<OptimisationEstimate, AppError> {
    require_licence()?;
    tauri::async_runtime::spawn_blocking(move || app::estimate_pdf(&PathBuf::from(path), &profile))
        .await
        .map_err(join_error)?
}

#[tauri::command]
async fn optimise_pdf(
    app_handle: AppHandle,
    state: State<'_, OptimisationState>,
    path: String,
    profile: String,
    output_path: String,
) -> Result<OptimisationResult, AppError> {
    require_licence()?;
    let token = CancellationToken::default();
    *state.0.lock().expect("optimisation state poisoned") = token.clone();
    let pdfium_library = app_handle
        .path()
        .resource_dir()
        .ok()
        .map(|directory| directory.join(pdfium_library_name()));
    tauri::async_runtime::spawn_blocking(move || {
        app::optimise_pdf_with_options(
            &PathBuf::from(path),
            &profile,
            &PathBuf::from(output_path),
            &token,
            pdfium_library.as_deref(),
            |stage| {
                let _ = app_handle.emit("optimisation-stage", stage);
            },
        )
    })
    .await
    .map_err(join_error)?
}

fn pdfium_library_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.dylib"
    }
}

#[tauri::command]
fn cancel_optimisation(state: State<'_, OptimisationState>) {
    state
        .0
        .lock()
        .expect("optimisation state poisoned")
        .cancel();
}

#[tauri::command]
fn get_licence_status() -> LicenceStatus {
    licensing::local_status()
}

#[tauri::command]
async fn activate_licence(licence_key: String) -> LicenceStatus {
    licensing::activate(licence_key).await
}

#[tauri::command]
async fn revalidate_licence() -> LicenceStatus {
    licensing::revalidate().await
}

#[tauri::command]
async fn deactivate_licence() -> LicenceStatus {
    licensing::deactivate().await
}

fn require_licence() -> Result<(), AppError> {
    licensing::require_active().map_err(|message| AppError {
        code: app::AppErrorCode::OptimisationFailed,
        message,
        detail: None,
    })
}

fn join_error(error: impl std::fmt::Display) -> AppError {
    AppError {
        code: app::AppErrorCode::OptimisationFailed,
        message: "The desktop worker stopped unexpectedly.".into(),
        detail: cfg!(debug_assertions).then(|| error.to_string()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(OptimisationState(Mutex::new(CancellationToken::default())))
        .invoke_handler(tauri::generate_handler![
            inspect_pdf,
            estimate_pdf,
            optimise_pdf,
            cancel_optimisation,
            get_licence_status,
            activate_licence,
            revalidate_licence,
            deactivate_licence
        ])
        .run(tauri::generate_context!())
        .expect("error while running NoBS PDF");
}
