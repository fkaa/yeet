use std::sync::Arc;

use axum::{Router, routing::get, extract::{WebSocketUpgrade, Path, Extension, ws::WebSocket}, response::IntoResponse, AddExtensionLayer};
use tracing::*;
use tracing_subscriber::{EnvFilter, FmtSubscriber};
use tokio::sync::mpsc::{self, Sender, Receiver};

use std::sync::RwLock;
use std::collections::HashMap;

type Channel = (Sender<ClientMessage>, Receiver<ClientMessage>);

struct SessionState {
    creator: Channel,
    participant: Channel,
}

impl SessionState {
    fn new() -> Self {
        let creator = mpsc::channel(50);
        let participant = mpsc::channel(50);

        SessionState {
            creator,
            participant,
        }
    }
}

struct AppData {
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
}

enum ServerMessage {
}

#[derive(Clone)]
enum ClientMessage {
    /// Someone has joined the session
    ParticipantJoin,

    /// Someone has left the session
    ParticipantLeave,

    CreateSession,

    /// An offer to upload a file
    FileOffer {
        name: String,
        size: u64
    },

    FileOfferResponse {
        answer: bool,
    },

    /// A new ICE candidate to be forwarded to the other participant
    NewIceCandidate,

    /// A SDP offer that the creator of the session will send
    SdpOffer { sdp: String },

    /// Answer from the other participant
    Answer { sdp: String },
}

pub async fn websocket_signalling(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| handle_websocket_signalling(socket, stream, data))
}

async fn handle_websocket_signalling(socket: WebSocket, stream: String, data: Arc<AppData>) {
    let mut sessions = data.sessions.write().unwrap();

    if let Some(session) = sessions.get(&stream) {
    } else {
        let state = SessionState::new();
        let creator = state.creator.clone();

        sessions.insert(stream, state);
    }
}

async fn negotiate(
    a: (Sender<ClientMessage>, Receiver<ClientMessage>),
    b: (Sender<ClientMessage>, Receiver<ClientMessage>)
) {

}

async fn start() -> anyhow::Result<()> {
    /*let data = Arc::new(AppData {
        stream_repo,
        client: client.clone(),
        stream_stat_sender,
    });*/

    let app = Router::new()
        .route("/signalling/:stream", get(websocket_signalling))
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
