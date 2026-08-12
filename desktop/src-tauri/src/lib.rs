use std::{path::PathBuf, sync::Mutex};

use pdfdoctor::app::{
    self, AppError, CancellationToken, DocumentSummary, OptimisationEstimate, OptimisationResult,
};
use tauri::{AppHandle, Emitter, Manager, State};

mod licensing;
use licensing::LicenceStatus;

struct OptimisationState(Mutex<CancellationToken>);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateStatus {
    current_version: String,
    latest_version: String,
    update_available: bool,
    download_page: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublicReleaseConfig {
    release_version: String,
}

#[tauri::command]
async fn check_for_updates() -> Result<UpdateStatus, AppError> {
    let base = option_env!("NOBS_LICENSE_API_URL").unwrap_or("https://nobs-pdf.com");
    let response = reqwest::Client::new()
        .get(format!("{}/api/config", base.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(update_error)?;
    if !response.status().is_success() {
        return Err(update_error(format!("HTTP {}", response.status())));
    }
    let release = response
        .json::<PublicReleaseConfig>()
        .await
        .map_err(update_error)?;
    let current = env!("CARGO_PKG_VERSION");
    Ok(UpdateStatus {
        current_version: current.into(),
        update_available: version_tuple(&release.release_version) > version_tuple(current),
        latest_version: release.release_version,
        download_page: base.into(),
    })
}

fn version_tuple(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.').map(|part| part.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn update_error(error: impl std::fmt::Display) -> AppError {
    AppError {
        code: app::AppErrorCode::OptimisationFailed,
        message: "NoBS PDF could not check for updates. Check your connection and try again."
            .into(),
        detail: cfg!(debug_assertions).then(|| error.to_string()),
    }
}

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
    tauri::async_runtime::spawn_blocking(move || {
        app::estimate_pdf_scale(&PathBuf::from(path), scale_percent)
    })
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
    let pdfium_library = packaged_pdfium_path(&app_handle)?;
    let token = CancellationToken::default();
    *state.0.lock().expect("optimisation state poisoned") = token.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app::optimise_pdf_scale_with_pdfium(
            &PathBuf::from(path),
            scale_percent,
            &PathBuf::from(output_path),
            &token,
            Some(&pdfium_library),
            |stage| {
                let _ = app_handle.emit("optimisation-stage", stage);
            },
        )
    })
    .await
    .map_err(join_error)?
}

fn packaged_pdfium_path(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    let resource_dir = app_handle.path().resource_dir().map_err(|error| AppError {
        code: app::AppErrorCode::OptimisationFailed,
        message: "NoBS PDF could not locate its PDF rendering component.".into(),
        detail: cfg!(debug_assertions).then(|| error.to_string()),
    })?;
    pdfium_path_in(&resource_dir)
}

fn pdfium_path_in(resource_dir: &std::path::Path) -> Result<PathBuf, AppError> {
    let filename = if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.dylib"
    };
    let path = resource_dir.join(filename);
    if !path.is_file() {
        return Err(AppError {
            code: app::AppErrorCode::OptimisationFailed,
            message: "NoBS PDF could not locate its PDF rendering component.".into(),
            detail: cfg!(debug_assertions).then(|| format!("missing {}", path.display())),
        });
    }
    Ok(path)
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
            deactivate_licence,
            check_for_updates
        ])
        .run(tauri::generate_context!())
        .expect("error while running NoBS PDF");
}

#[cfg(test)]
mod licensing_boundary_tests {
    use std::fs;

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

    #[test]
    fn packaged_pdfium_is_resolved_from_the_runtime_resource_directory() {
        let directory = tempfile::tempdir().unwrap();
        let filename = if cfg!(target_os = "windows") {
            "pdfium.dll"
        } else {
            "libpdfium.dylib"
        };
        let expected = directory.path().join(filename);
        fs::write(&expected, b"test library").unwrap();
        assert_eq!(super::pdfium_path_in(directory.path()).unwrap(), expected);
    }

    #[test]
    fn update_versions_are_compared_numerically() {
        assert!(super::version_tuple("1.1.0") > super::version_tuple("1.0.9"));
        assert!(super::version_tuple("2.0.0") > super::version_tuple("1.99.99"));
        assert_eq!(super::version_tuple("1.0.0"), (1, 0, 0));
    }
}
