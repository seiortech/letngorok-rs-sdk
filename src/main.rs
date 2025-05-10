use letngorok_rs_sdk::TunnelClient;
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("Setting default tracing subscriber failed.");

    let token = "set-your-token-here";
    let local_port = "set-your-local-port-here";

    let client = match TunnelClient::new(None, token.to_string()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Failed to create TunnelClient: {:?}", e);
            return;
        }
    };
    tracing::info!("TunnelClient created.");

    if let Err(e) = client.start(local_port.to_string(), None).await {
        tracing::error!("Failed to start tunnel for port {}: {:?}", local_port, e);
        return;
    }
    tracing::info!(
        "Tunnel initiation for local port {} requested. Check logs for connection status.",
        local_port
    );

    tracing::info!("Tunnel is active. Press Ctrl+C to stop and exit.");
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to listen for Ctrl+C signal");

    tracing::info!("Ctrl+C received. Shutting down tunnel(s)...");
    if let Err(e) = client.stop_all().await {
        tracing::error!("Error during tunnel shutdown: {:?}", e);
    }

    tracing::info!("Application has exited.");
}
