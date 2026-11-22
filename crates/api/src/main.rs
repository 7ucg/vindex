//! Standalone-Daemon-Einstieg für die vindex API.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let addr = std::env::var("VINDEX_API_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    vindex_api::serve(&addr).await
}
