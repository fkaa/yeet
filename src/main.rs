use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, Path, WebSocketUpgrade,
    },
    response::IntoResponse,
    routing::get,
    AddExtensionLayer, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Receiver, Sender};
use futures_util::{SinkExt, stream::StreamExt};
use tracing::*;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use std::collections::HashMap;
use std::sync::RwLock;

type Channel = (Sender<ClientMessage>, Receiver<ClientMessage>);

/// Contains the channel receivers that incoming websocket connections
/// should use.
struct SessionState {
    bob: Option<Receiver<ClientMessage>>,
    bob_sender: Sender<ClientMessage>,
    alice: Option<Receiver<ClientMessage>>,
    alice_sender: Sender<ClientMessage>,
}

impl SessionState {
    fn new() -> Self {
        let (bob_sender, bob) = mpsc::channel(50);
        let (alice_sender, alice) = mpsc::channel(50);

        SessionState {
            bob: Some(bob),
            bob_sender,
            alice: Some(alice),
            alice_sender,
        }
    }

    fn return_channel(&mut self, rx: Receiver<ClientMessage>) -> bool {
        if self.bob.is_none() {
            self.bob = Some(rx);
        } else if self.alice.is_none() {
            self.alice = Some(rx);
        }

        self.bob.is_some() && self.alice.is_some()
    }

    fn take(&mut self) -> Option<(Sender<ClientMessage>, Receiver<ClientMessage>)> {
        self.bob
            .take()
            .map(|rx| (self.bob_sender.clone(), rx))
            .or_else(|| self.alice.take().map(|rx| (self.alice_sender.clone(), rx)))
    }
}

struct AppData {
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
}

enum ServerMessage {}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum ClientMessage {
    /// Someone has joined the session
    ParticipantJoin,

    /// Someone has left the session
    ParticipantLeave,

    CreateSession,

    /// An offer to upload a file
    FileOffer {
        name: String,
        size: u64,
    },

    FileOfferResponse {
        answer: bool,
    },

    /// A new ICE candidate to be forwarded to the other participant
    NewIceCandidate,

    /// A SDP offer that the creator of the session will send
    SdpOffer {
        sdp: String,
    },

    /// Answer from the other participant
    Answer {
        sdp: String,
    },
}

async fn websocket_signalling(
    ws: WebSocketUpgrade,
    Path(stream): Path<String>,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    debug!("Received websocket request for '{}'", stream);

    ws.on_upgrade(move |socket| handle_websocket_signalling(socket, stream, data))
}

async fn signalling_loop(
    mut socket: WebSocket,
    stream: &str,
    tx: Sender<ClientMessage>,
    rx: &mut Receiver<ClientMessage>,
) -> anyhow::Result<()> {
    debug!("Connected to session '{stream}'");

    let (mut ws_send, mut ws_recv) = socket.split();

    tokio::select! {
        res = async {
            loop {
                let msg = rx.recv().await;

                match msg {
                    Some(msg) => {
                        let j = serde_json::to_string(&msg)?;

                        ws_send.send(Message::Text(j)).await?;
                    },
                    _ => {},
                }
            }
        } => res,

        res = async {
            loop {
                let msg = ws_recv.next().await;

                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let msg = serde_json::from_str(&txt)?;

                        tx.send(msg).await?;
                    },
                    Some(Ok(Message::Close(close))) => {
                        anyhow::bail!("WebSocket closed by remote: {close:?}")
                    },
                    Some(Err(e)) => Err(e)?,
                    None => anyhow::bail!("No more message from WebSocket"),
                    _ => {},
                }
            }
        } => res,
    }
}

async fn handle_websocket_signalling(socket: WebSocket, stream: String, data: Arc<AppData>) {
    let mut channels = {
        let mut sessions = data.sessions.write().unwrap();

        if let Some(session) = sessions.get_mut(&stream) {
            session.take()
        } else {
            let mut state = SessionState::new();

            debug!("Created new session for '{stream}'");

            let (tx, rx) = state.take().unwrap();

            sessions.insert(stream.clone(), state);

            Some((tx, rx))
        }
    };

    if let Some((tx, mut rx)) = channels {
        if let Err(e) = signalling_loop(socket, &stream, tx, &mut rx).await {
            warn!("Error while handling signalling for {stream}: {e}");
        }

        let mut sessions = data.sessions.write().unwrap();
        let session = sessions.get_mut(&stream).unwrap();
        let is_empty = session.return_channel(rx);

        if is_empty {
            debug!("Session '{stream}' is empty, removing");

            sessions.remove(&stream);
        }
    } else {
        debug!("Session '{stream}' is already full");
    }

    debug!("Left session '{stream}'");

    // socket.close().await;
}

async fn start() -> anyhow::Result<()> {
    let data = Arc::new(AppData {
        sessions: Arc::new(RwLock::new(HashMap::new())),
    });

    let app = Router::new()
        .route("/signalling/:stream", get(websocket_signalling))
        .layer(AddExtensionLayer::new(data.clone()));

    let addr = "127.0.0.1:8080".parse().unwrap();
    hyper::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

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
