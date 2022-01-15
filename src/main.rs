use std::sync::Arc;

use axum::{Router, routing::get, extract::{WebSocketUpgrade, Path, Extension, ws::WebSocket}, response::IntoResponse, AddExtensionLayer};
use tracing::*;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

struct AppData;

pub async fn websocket_video(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| handle_websocket_signalling(socket, stream, data))
}

async fn handle_websocket_signalling(socket: WebSocket, stream: String, data: Arc<AppData>) {
}

async fn start() -> anyhow::Result<()> {
    /*let data = Arc::new(AppData {
        stream_repo,
        client: client.clone(),
        stream_stat_sender,
    });*/

    let app = Router::new()
        .route("/signalling/:stream", get(websocket_video))
        .layer(AddExtensionLayer::new(data.clone()));


    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("debug"))
        .unwrap();

    let subscriber = FmtSubscriber::builder().with_env_filter(filter).finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { start().await })?;

    Ok(())
}
