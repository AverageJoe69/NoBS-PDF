fn main() {
    let profile = std::env::var("PROFILE").unwrap_or_default();
    if profile == "release" {
        let api_url = std::env::var("NOBS_LICENSE_API_URL").unwrap_or_else(|_| {
            panic!("NOBS_LICENSE_API_URL must be set explicitly for release builds")
        });
        let lower = api_url.to_ascii_lowercase();
        if !lower.starts_with("https://")
            || lower.contains("localhost")
            || lower.contains("127.0.0.1")
            || lower.contains(".test")
        {
            panic!("release NOBS_LICENSE_API_URL must be a production HTTPS origin");
        }
        println!("cargo:rustc-env=NOBS_LICENSE_API_URL={api_url}");
    }
    println!("cargo:rerun-if-env-changed=NOBS_LICENSE_API_URL");
    tauri_build::build()
}
