use std::{path::PathBuf, sync::Mutex};

use pdfdoctor::app::{
    self, AppError, CancellationToken, DocumentSummary, OptimisationEstimate, OptimisationResult,
};
use tauri::{AppHandle, Emitter, State};

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
async fn estimate_pdf(path: String, scale_percent: u8) -> Result<OptimisationEstimate, AppError> {
    require_licence()?;
    tauri::async_runtime::spawn_blocking(move || app::estimate_pdf_scale(&PathBuf::from(path), scale_percent))
        .await
        .map_err(join_error)?
}

#[tauri::command]
async fn optimise_pdf(
    app_handle: AppHandle,
    state: State<'_, OptimisationState>,
    path: String,
    scale_percent: u8,
    output_path: String,
) -> Result<OptimisationResult, AppError> {
    require_licence()?;
    let token = CancellationToken::default();
    *state.0.lock().expect("optimisation state poisoned") = token.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app::optimise_pdf_scale_with_options(
            &PathBuf::from(path),
            scale_percent,
            &PathBuf::from(output_path),
            &token,
            |stage| {
                let _ = app_handle.emit("optimisation-stage", stage);
            },
        )
    })
    .await
    .map_err(join_error)?
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

#[cfg(test)]
mod licensing_boundary_tests {
    #[test]
    fn pdf_commands_only_use_the_local_licence_gate() {
        let source = include_str!("lib.rs");
        for (function, next) in [
            (
                "async fn inspect_pdf",
                "#[tauri::command]\nasync fn estimate_pdf",
            ),
            (
                "async fn estimate_pdf",
                "#[tauri::command]\nasync fn optimise_pdf",
            ),
            (
                "async fn optimise_pdf",
                "#[tauri::command]\nfn cancel_optimisation",
            ),
        ] {
            let body = source
                .split_once(function)
                .unwrap()
                .1
                .split_once(next)
                .unwrap()
                .0;
            assert!(body.contains("require_licence()?"));
            assert!(!body.contains("revalidate"));
            assert!(!body.contains("activate_licence"));
            assert!(!body.contains("api/license"));
            assert!(!body.contains("reqwest"));
        }
    }
}
