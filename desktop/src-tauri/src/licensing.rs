use std::time::Duration;

use keyring::{Entry, Error as KeyringError};
use reqwest::{redirect::Policy, Client, StatusCode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SERVICE: &str = "com.nobspdf.desktop";
const CREDENTIAL_ACCOUNT: &str = "licence-activation";
const DEVICE_ACCOUNT: &str = "device-identifier";
const DEFAULT_API_URL: &str = "https://nobs-pdf.com";
const DAY_SECONDS: i64 = 24 * 60 * 60;
const NORMAL_VERIFICATION_SECONDS: i64 = 30 * DAY_SECONDS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LicenceState {
    NotActivated,
    Active,
    Invalid,
    Revoked,
    Expired,
    ActivationLimitReached,
    NetworkError,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenceStatus {
    pub state: LicenceState,
    pub message: Option<String>,
    pub licence_key: Option<String>,
    pub device_name: String,
    pub locally_activated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredCredential {
    licence_key: String,
    activation_id: String,
    activation_token: String,
    release_version: String,
    platform: String,
    state: LicenceState,
    #[serde(default)]
    last_verified_at: Option<i64>,
    #[serde(default)]
    last_verification_attempt_at: Option<i64>,
    #[serde(default)]
    verification_failure_count: u32,
}

#[derive(Serialize)]
struct ActivateRequest<'a> {
    license_key: &'a str,
    device_identifier: &'a str,
    app_version: &'a str,
    platform: &'a str,
}

#[derive(Serialize)]
struct ActivationRequest<'a> {
    activation_id: &'a str,
}

#[derive(Deserialize)]
struct ApiResponse {
    #[serde(default)]
    valid: bool,
    state: LicenceState,
    message: Option<String>,
    activation_id: Option<String>,
    activation_token: Option<String>,
    release_version: Option<String>,
    platform: Option<String>,
}

fn credential_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, CREDENTIAL_ACCOUNT)
        .map_err(|_| "The system credential store is unavailable.".into())
}

fn device_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, DEVICE_ACCOUNT)
        .map_err(|_| "The system credential store is unavailable.".into())
}

fn read_credential() -> Result<Option<StoredCredential>, String> {
    match credential_entry()?.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map(Some)
            .map_err(|_| "The stored activation credential is unreadable.".into()),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(_) => Err("The system credential store could not be read.".into()),
    }
}

fn write_credential(credential: &StoredCredential) -> Result<(), String> {
    let value = serde_json::to_string(credential)
        .map_err(|_| "The activation credential could not be encoded.".to_string())?;
    credential_entry()?
        .set_password(&value)
        .map_err(|_| "The activation could not be saved in the system credential store.".into())
}

fn delete_credential() -> Result<(), String> {
    match credential_entry()?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(_) => {
            Err("The activation could not be removed from the system credential store.".into())
        }
    }
}

fn device_identifier() -> Result<String, String> {
    match device_entry()?.get_password() {
        Ok(value) if Uuid::parse_str(&value).is_ok() => Ok(value),
        Ok(_) | Err(KeyringError::NoEntry) => {
            let value = Uuid::new_v4().to_string();
            device_entry()?
                .set_password(&value)
                .map_err(|_| "The device identifier could not be saved securely.".to_string())?;
            Ok(value)
        }
        Err(_) => {
            Err("The device identifier could not be read from the system credential store.".into())
        }
    }
}

fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else {
        "macos"
    }
}

fn device_name() -> String {
    if cfg!(target_os = "windows") {
        "This PC".into()
    } else {
        "This Mac".into()
    }
}

fn api_url() -> &'static str {
    option_env!("NOBS_LICENSE_API_URL")
        .unwrap_or(DEFAULT_API_URL)
        .trim_end_matches('/')
}

fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(Duration::from_secs(12))
        .redirect(Policy::none())
        .user_agent(concat!("NoBS-PDF/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "The licensing service could not be prepared.".into())
}

fn now_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn verification_jitter_seconds(activation_id: &str) -> i64 {
    let value = activation_id.bytes().fold(0_u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as u64)
    });
    (value % (2 * DAY_SECONDS as u64 + 1)) as i64 - DAY_SECONDS
}

fn retry_seconds(failures: u32) -> i64 {
    match failures {
        0 | 1 => DAY_SECONDS,
        2 => 3 * DAY_SECONDS,
        _ => 7 * DAY_SECONDS,
    }
}

fn verification_due(credential: &StoredCredential, now: i64) -> bool {
    if credential.state != LicenceState::Active {
        return false;
    }
    if credential.verification_failure_count > 0 {
        return credential
            .last_verification_attempt_at
            .map(|attempt| {
                now.saturating_sub(attempt) >= retry_seconds(credential.verification_failure_count)
            })
            .unwrap_or(true);
    }
    credential
        .last_verified_at
        .map(|verified| {
            now.saturating_sub(verified)
                >= NORMAL_VERIFICATION_SECONDS
                    + verification_jitter_seconds(&credential.activation_id)
        })
        .unwrap_or(true)
}

fn usable_offline(credential: &StoredCredential, _now: i64) -> bool {
    credential.state == LicenceState::Active
}

pub fn normalize_licence_key(value: &str) -> Option<String> {
    let compact: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_uppercase)
        .collect();
    let body = compact.strip_prefix("NOBS").unwrap_or(&compact);
    if body.len() != 16 {
        return None;
    }
    Some(format!(
        "NOBS-{}-{}-{}-{}",
        &body[0..4],
        &body[4..8],
        &body[8..12],
        &body[12..16]
    ))
}

pub fn local_status() -> LicenceStatus {
    match read_credential() {
        Ok(Some(credential)) => {
            let usable = usable_offline(&credential, now_timestamp());
            let state = credential.state.clone();
            LicenceStatus {
                locally_activated: usable,
                message: match state {
                    LicenceState::Revoked => {
                        Some("This licence has been revoked. Please contact support.".into())
                    }
                    _ => None,
                },
                state,
                licence_key: Some(credential.licence_key),
                device_name: device_name(),
            }
        }
        Ok(None) => LicenceStatus {
            state: LicenceState::NotActivated,
            message: None,
            licence_key: None,
            device_name: device_name(),
            locally_activated: false,
        },
        Err(message) => LicenceStatus {
            state: LicenceState::Invalid,
            message: Some(message),
            licence_key: None,
            device_name: device_name(),
            locally_activated: false,
        },
    }
}

pub fn require_active() -> Result<(), String> {
    match read_credential()? {
        Some(credential) if usable_offline(&credential, now_timestamp()) => Ok(()),
        _ => Err("NoBS PDF must be activated before processing documents.".into()),
    }
}

pub async fn activate(value: String) -> LicenceStatus {
    let Some(licence_key) = normalize_licence_key(&value) else {
        return status(
            LicenceState::Invalid,
            "Enter a valid NoBS PDF licence key.",
            false,
        );
    };
    let device = match device_identifier() {
        Ok(value) => value,
        Err(message) => return status(LicenceState::Invalid, &message, false),
    };
    let response = client()
        .and_then(|client| {
            Ok(client
                .post(format!("{}/api/license/activate", api_url()))
                .json(&ActivateRequest {
                    license_key: &licence_key,
                    device_identifier: &device,
                    app_version: env!("CARGO_PKG_VERSION"),
                    platform: platform(),
                }))
        })
        .map_err(|message| message);
    let response = match response {
        Ok(request) => request.send().await,
        Err(message) => return status(LicenceState::Invalid, &message, false),
    };
    let response = match response {
        Ok(value) => value,
        Err(_) => {
            return status(
                LicenceState::NetworkError,
                "An internet connection is required for first activation.",
                false,
            )
        }
    };
    let code = response.status();
    let body = match response.json::<ApiResponse>().await {
        Ok(value) => value,
        Err(_) => {
            return status(
                LicenceState::Invalid,
                "The licensing service returned an invalid response.",
                false,
            )
        }
    };
    if code == StatusCode::CREATED && body.valid && body.state == LicenceState::Active {
        let now = now_timestamp();
        let credential = active_credential_from_response(&licence_key, body, now);
        if credential.activation_id.is_empty() || credential.activation_token.is_empty() {
            return status(
                LicenceState::Invalid,
                "The licensing service returned an incomplete activation.",
                false,
            );
        }
        if let Err(message) = write_credential(&credential) {
            return status(LicenceState::Invalid, &message, false);
        }
        return LicenceStatus {
            state: LicenceState::Active,
            message: None,
            licence_key: Some(licence_key),
            device_name: device_name(),
            locally_activated: true,
        };
    }
    LicenceStatus {
        state: body.state,
        message: body.message,
        licence_key: Some(licence_key),
        device_name: device_name(),
        locally_activated: false,
    }
}

fn active_credential_from_response(
    licence_key: &str,
    body: ApiResponse,
    now: i64,
) -> StoredCredential {
    StoredCredential {
        licence_key: licence_key.into(),
        activation_id: body.activation_id.unwrap_or_default(),
        activation_token: body.activation_token.unwrap_or_default(),
        release_version: body.release_version.unwrap_or_default(),
        platform: body.platform.unwrap_or_else(|| platform().into()),
        state: LicenceState::Active,
        last_verified_at: Some(now),
        last_verification_attempt_at: Some(now),
        verification_failure_count: 0,
    }
}

pub async fn revalidate() -> LicenceStatus {
    let Some(credential) = read_credential().ok().flatten() else {
        return local_status();
    };
    let http = match client() {
        Ok(client) => client,
        Err(_) => return network_status(&credential),
    };
    let (credential, status, attempted) =
        revalidate_credential(credential, now_timestamp(), &http, api_url()).await;
    if attempted {
        let _ = write_credential(&credential);
    }
    status
}

async fn revalidate_credential(
    mut credential: StoredCredential,
    now: i64,
    http: &Client,
    base_url: &str,
) -> (StoredCredential, LicenceStatus, bool) {
    if !verification_due(&credential, now) {
        let status = credential_status_at(&credential, now);
        return (credential, status, false);
    }

    credential.last_verification_attempt_at = Some(now);
    let response = http
        .post(format!(
            "{}/api/license/verify",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(&credential.activation_token)
        .json(&ActivationRequest {
            activation_id: &credential.activation_id,
        })
        .send()
        .await;

    let Ok(response) = response else {
        credential.verification_failure_count =
            credential.verification_failure_count.saturating_add(1);
        let status = network_status_at(&credential, now);
        return (credential, status, true);
    };
    let code = response.status();
    let body = response.json::<ApiResponse>().await.ok();

    match (code, body) {
        (StatusCode::OK, Some(body)) if body.valid && body.state == LicenceState::Active => {
            credential.last_verified_at = Some(now);
            credential.verification_failure_count = 0;
            let status = credential_status_at(&credential, now);
            (credential, status, true)
        }
        (StatusCode::UNAUTHORIZED, Some(body))
            if !body.valid && body.state == LicenceState::Invalid =>
        {
            credential.state = LicenceState::Invalid;
            let status = protocol_status(&credential, body.message);
            (credential, status, true)
        }
        (StatusCode::FORBIDDEN, Some(body))
            if !body.valid && body.state == LicenceState::Revoked =>
        {
            credential.state = LicenceState::Revoked;
            let status = protocol_status(&credential, body.message);
            (credential, status, true)
        }
        _ => {
            credential.verification_failure_count =
                credential.verification_failure_count.saturating_add(1);
            let status = network_status_at(&credential, now);
            (credential, status, true)
        }
    }
}

#[cfg(test)]
fn credential_status(credential: &StoredCredential) -> LicenceStatus {
    credential_status_at(credential, now_timestamp())
}

fn credential_status_at(credential: &StoredCredential, now: i64) -> LicenceStatus {
    LicenceStatus {
        state: credential.state.clone(),
        message: None,
        licence_key: Some(credential.licence_key.clone()),
        device_name: device_name(),
        locally_activated: usable_offline(credential, now),
    }
}

fn protocol_status(credential: &StoredCredential, message: Option<String>) -> LicenceStatus {
    LicenceStatus {
        state: credential.state.clone(),
        message,
        licence_key: Some(credential.licence_key.clone()),
        device_name: device_name(),
        locally_activated: false,
    }
}

pub async fn deactivate() -> LicenceStatus {
    let Some(credential) = read_credential().ok().flatten() else {
        return local_status();
    };
    let request = match client() {
        Ok(client) => client
            .post(format!("{}/api/license/deactivate", api_url()))
            .bearer_auth(&credential.activation_token)
            .json(&ActivationRequest {
                activation_id: &credential.activation_id,
            }),
        Err(_) => return network_status(&credential),
    };
    match request.send().await {
        Ok(response) if response.status().is_success() => match delete_credential() {
            Ok(()) => local_status(),
            Err(message) => status(LicenceState::Invalid, &message, true),
        },
        Ok(response) => {
            let body = response.json::<ApiResponse>().await.ok();
            status(
                body.as_ref()
                    .map(|value| value.state.clone())
                    .unwrap_or(LicenceState::Invalid),
                body.and_then(|value| value.message)
                    .as_deref()
                    .unwrap_or("This device could not be deactivated."),
                true,
            )
        }
        Err(_) => network_status(&credential),
    }
}

fn network_status(credential: &StoredCredential) -> LicenceStatus {
    network_status_at(credential, now_timestamp())
}

fn network_status_at(credential: &StoredCredential, now: i64) -> LicenceStatus {
    let usable = usable_offline(credential, now);
    LicenceStatus {
        state: LicenceState::NetworkError,
        message: Some("NoBS PDF is offline. Your existing activation remains usable.".into()),
        licence_key: Some(credential.licence_key.clone()),
        device_name: device_name(),
        locally_activated: usable,
    }
}

fn status(state: LicenceState, message: &str, locally_activated: bool) -> LicenceStatus {
    LicenceStatus {
        state,
        message: Some(message.into()),
        licence_key: None,
        device_name: device_name(),
        locally_activated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        thread,
    };

    fn active_credential(now: i64) -> StoredCredential {
        StoredCredential {
            licence_key: "NOBS-AB12-CD34-EF56-7890".into(),
            activation_id: "act_test".into(),
            activation_token: "secret".into(),
            release_version: "1.0.0".into(),
            platform: "macos".into(),
            state: LicenceState::Active,
            last_verified_at: Some(now),
            last_verification_attempt_at: Some(now),
            verification_failure_count: 0,
        }
    }

    fn test_client(timeout: Duration) -> Client {
        Client::builder()
            .timeout(timeout)
            .redirect(Policy::none())
            .build()
            .unwrap()
    }

    fn server(
        status: u16,
        content_type: &str,
        body: &str,
        delay: Duration,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let requests = count.clone();
        let content_type = content_type.to_string();
        let body = body.to_string();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                requests.fetch_add(1, Ordering::SeqCst);
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request);
                thread::sleep(delay);
                let reason = match status {
                    200 => "OK",
                    302 => "Found",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    429 => "Too Many Requests",
                    _ => "Internal Server Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nLocation: /website\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (format!("http://{address}"), count)
    }

    fn run(
        credential: StoredCredential,
        now: i64,
        http: &Client,
        url: &str,
    ) -> (StoredCredential, LicenceStatus, bool) {
        tauri::async_runtime::block_on(revalidate_credential(credential, now, http, url))
    }

    #[test]
    fn normalises_spaces_hyphens_and_case() {
        assert_eq!(
            normalize_licence_key(" nobs ab12-cd34 ef56 7890 ").as_deref(),
            Some("NOBS-AB12-CD34-EF56-7890")
        );
        assert_eq!(normalize_licence_key("short"), None);
    }

    #[test]
    fn network_failure_preserves_an_active_local_installation() {
        let credential = active_credential(1_000_000);
        let result = network_status_at(&credential, 1_000_000);
        assert_eq!(result.state, LicenceState::NetworkError);
        assert!(result.locally_activated);
    }

    #[test]
    fn fresh_activation_sets_verification_timestamps() {
        let now = 1_700_000_000;
        let credential = active_credential_from_response(
            "NOBS-AB12-CD34-EF56-7890",
            ApiResponse {
                valid: true,
                state: LicenceState::Active,
                message: None,
                activation_id: Some("act_fresh".into()),
                activation_token: Some("token".into()),
                release_version: Some("1.0.0".into()),
                platform: Some("macos".into()),
            },
            now,
        );
        assert_eq!(credential.last_verified_at, Some(now));
        assert_eq!(credential.last_verification_attempt_at, Some(now));
        assert_eq!(credential.verification_failure_count, 0);
        assert_eq!(credential.state, LicenceState::Active);
    }

    #[test]
    fn legacy_perpetual_credential_remains_usable_offline() {
        let legacy = r#"{"licence_key":"NOBS-AB12-CD34-EF56-7890","activation_id":"act_legacy","activation_token":"token","release_version":"1.0.0","platform":"macos","state":"ACTIVE"}"#;
        let credential: StoredCredential = serde_json::from_str(legacy).unwrap();
        assert_eq!(credential.state, LicenceState::Active);
        assert_eq!(credential.last_verified_at, None);
        assert_eq!(credential.last_verification_attempt_at, None);
        assert_eq!(credential.verification_failure_count, 0);
        assert!(credential_status(&credential).locally_activated);
    }

    #[test]
    fn perpetual_credential_is_usable_offline_indefinitely() {
        let now = 1_700_000_000;
        let credential = active_credential(now);
        assert!(usable_offline(&credential, now));
        assert!(usable_offline(&credential, now + 20 * 365 * DAY_SECONDS));
    }

    #[test]
    fn launch_before_interval_makes_no_http_request() {
        let now = 1_700_000_000;
        let (url, requests) = server(
            500,
            "application/json",
            r#"{"state":"INVALID"}"#,
            Duration::ZERO,
        );
        let (_, status, attempted) = run(
            active_credential(now),
            now + 20 * DAY_SECONDS,
            &test_client(Duration::from_secs(1)),
            &url,
        );
        assert!(!attempted);
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        assert_eq!(status.state, LicenceState::Active);
    }

    #[test]
    fn due_credential_attempts_once_and_active_response_updates_timestamp() {
        let now = 1_700_000_000;
        let body = r#"{"valid":true,"state":"ACTIVE","activation_id":"act_test","release_version":"1.0.0","platform":"macos"}"#;
        let (url, requests) = server(200, "application/json", body, Duration::ZERO);
        let credential = active_credential(now - 32 * DAY_SECONDS);
        let (updated, status, attempted) =
            run(credential, now, &test_client(Duration::from_secs(1)), &url);
        assert!(attempted);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(updated.last_verified_at, Some(now));
        assert_eq!(updated.verification_failure_count, 0);
        assert_eq!(status.state, LicenceState::Active);
    }

    #[test]
    fn transient_http_and_payload_failures_leave_active() {
        let cases = [
            (500, "application/json", r#"{"error":"unavailable"}"#),
            (404, "application/json", r#"{"error":"missing"}"#),
            (429, "application/json", r#"{"error":"limited"}"#),
            (302, "text/html", "redirect"),
            (500, "application/json", r#"{"state":"INVALID"}"#),
            (
                200,
                "application/json",
                r#"{"valid":false,"state":"INVALID"}"#,
            ),
            (
                200,
                "application/json",
                r#"{"valid":false,"state":"REVOKED"}"#,
            ),
            (
                401,
                "application/json",
                r#"{"valid":false,"state":"REVOKED"}"#,
            ),
            (
                403,
                "application/json",
                r#"{"valid":false,"state":"INVALID"}"#,
            ),
            (200, "text/html", "<html>maintenance</html>"),
            (200, "application/json", "{malformed"),
        ];
        for (status_code, content_type, body) in cases {
            let now = 1_700_000_000;
            let (url, requests) = server(status_code, content_type, body, Duration::ZERO);
            let credential = active_credential(now - 32 * DAY_SECONDS);
            let (updated, result, attempted) =
                run(credential, now, &test_client(Duration::from_secs(1)), &url);
            assert!(attempted, "status {status_code}");
            assert_eq!(requests.load(Ordering::SeqCst), 1, "status {status_code}");
            assert_eq!(updated.state, LicenceState::Active, "status {status_code}");
            assert!(result.locally_activated, "status {status_code}");
            assert_eq!(
                updated.verification_failure_count, 1,
                "status {status_code}"
            );
        }
    }

    #[test]
    fn timeout_and_connection_failure_leave_active() {
        let now = 1_700_000_000;
        let (url, _) = server(
            200,
            "application/json",
            r#"{"valid":true,"state":"ACTIVE"}"#,
            Duration::from_millis(200),
        );
        let credential = active_credential(now - 32 * DAY_SECONDS);
        let (timed_out, result, _) = run(
            credential,
            now,
            &test_client(Duration::from_millis(20)),
            &url,
        );
        assert_eq!(timed_out.state, LicenceState::Active);
        assert!(result.locally_activated);

        let credential = active_credential(now - 32 * DAY_SECONDS);
        let (disconnected, result, _) = run(
            credential,
            now,
            &test_client(Duration::from_millis(50)),
            "http://127.0.0.1:9",
        );
        assert_eq!(disconnected.state, LicenceState::Active);
        assert!(result.locally_activated);
    }

    #[test]
    fn only_expected_status_and_state_pairs_invalidate() {
        let now = 1_700_000_000;
        let cases = [
            (
                401,
                r#"{"valid":false,"state":"INVALID","message":"Invalid."}"#,
                LicenceState::Invalid,
            ),
            (
                403,
                r#"{"valid":false,"state":"REVOKED","message":"Revoked."}"#,
                LicenceState::Revoked,
            ),
        ];
        for (code, body, expected) in cases {
            let (url, _) = server(code, "application/json", body, Duration::ZERO);
            let credential = active_credential(now - 32 * DAY_SECONDS);
            let (updated, status, _) =
                run(credential, now, &test_client(Duration::from_secs(1)), &url);
            assert_eq!(updated.state, expected);
            assert_eq!(status.state, expected);
            assert!(!status.locally_activated);
        }

        let (url, _) = server(
            403,
            "application/json",
            r#"{"valid":false,"state":"EXPIRED","message":"Expired."}"#,
            Duration::ZERO,
        );
        let credential = active_credential(now - 32 * DAY_SECONDS);
        let (updated, status, _) = run(credential, now, &test_client(Duration::from_secs(1)), &url);
        assert_eq!(updated.state, LicenceState::Active);
        assert_eq!(status.state, LicenceState::NetworkError);
        assert!(status.locally_activated);
    }

    #[test]
    fn failed_verification_obeys_retry_schedule() {
        let now = 1_700_000_000;
        let (url, _) = server(
            500,
            "application/json",
            r#"{"error":"down"}"#,
            Duration::ZERO,
        );
        let credential = active_credential(now - 32 * DAY_SECONDS);
        let (failed, _, _) = run(credential, now, &test_client(Duration::from_secs(1)), &url);
        assert_eq!(failed.verification_failure_count, 1);

        let (url, requests) = server(
            200,
            "application/json",
            r#"{"valid":true,"state":"ACTIVE"}"#,
            Duration::ZERO,
        );
        let (_, _, attempted) = run(
            failed.clone(),
            now + DAY_SECONDS - 1,
            &test_client(Duration::from_secs(1)),
            &url,
        );
        assert!(!attempted);
        assert_eq!(requests.load(Ordering::SeqCst), 0);

        let (url, requests) = server(
            500,
            "application/json",
            r#"{"error":"down"}"#,
            Duration::ZERO,
        );
        let (failed_twice, _, attempted) = run(
            failed,
            now + DAY_SECONDS,
            &test_client(Duration::from_secs(1)),
            &url,
        );
        assert!(attempted);
        assert_eq!(requests.load(Ordering::SeqCst), 1);
        assert_eq!(failed_twice.verification_failure_count, 2);
        assert!(!verification_due(&failed_twice, now + 4 * DAY_SECONDS - 1));
        assert!(verification_due(&failed_twice, now + 4 * DAY_SECONDS));
    }
}
