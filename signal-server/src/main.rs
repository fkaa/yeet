use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, Path, WebSocketUpgrade,
    },
    response::{IntoResponse, Response},
    routing::get,
    AddExtensionLayer, Router,
};
use futures_util::{stream::StreamExt, SinkExt};
use hyper::StatusCode;
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::RwLock;
use tokio::sync::{
    broadcast::{self},
    oneshot,
};
use tracing::*;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use std::collections::HashMap;
use std::sync::Mutex;

/// Contains the channel receivers that incoming websocket connections
/// should use.
struct SessionState {
    file_name: String,
    file_size: u64,
    sender_tx: broadcast::Sender<ClientMessage>,
    receiver_tx: broadcast::Sender<ClientMessage>,

    cancellation_tx: broadcast::Sender<()>,
    cancellation_rx: broadcast::Receiver<()>,
    participants: Mutex<(bool, bool)>,
    // bob: Option<Receiver<ClientMessage>>,
    // bob_sender: Sender<ClientMessage>,
    // alice: Option<Receiver<ClientMessage>>,
    // alice_sender: Sender<ClientMessage>,
}

impl SessionState {
    fn new(file_name: String, file_size: u64) -> Self {
        let (sender_tx, _) = broadcast::channel(50);
        let (receiver_tx, _) = broadcast::channel(50);

        let (cancellation_tx, cancellation_rx) = broadcast::channel(1);

        SessionState {
            file_name,
            file_size,
            sender_tx,
            receiver_tx,
            cancellation_tx,
            cancellation_rx,
            participants: Mutex::new((false, false)),
        }
    }

    fn get_sender(
        &self,
    ) -> Option<(
        broadcast::Sender<ClientMessage>,
        broadcast::Receiver<ClientMessage>,
    )> {
        let mut p = self.participants.lock().unwrap();

        if p.0 {
            None
        } else {
            (*p).0 = true;

            let tx = self.receiver_tx.clone();
            let rx = self.sender_tx.subscribe();

            Some((tx, rx))
        }
    }

    fn return_receiver(&self) {
        let mut p = self.participants.lock().unwrap();
        (*p).1 = false;
    }

    fn get_receiver(
        &self,
    ) -> Option<(
        broadcast::Sender<ClientMessage>,
        broadcast::Receiver<ClientMessage>,
    )> {
        let mut p = self.participants.lock().unwrap();

        if p.1 {
            None
        } else {
            (*p).1 = true;

            let tx = self.sender_tx.clone();
            let rx = self.receiver_tx.subscribe();

            Some((tx, rx))
        }
    }
}

struct AppData {
    sessions: Arc<RwLock<HashMap<String, SessionState>>>,
}

enum ServerMessage {}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum ClientMessage {
    /// An offer to upload a file
    UploadFile {
        name: String,
        size: u64,
    },

    /// Response to file upload
    UploadFileResponse {
        link: String,
    },

    JoinSession {
        id: String,
    },

    /// Information about a session
    SessionInfo {
        file_name: String,
        size: u64,
    },

    /// Someone has joined the session
    ParticipantJoin,

    /// Someone has left the session
    ParticipantLeave,

    CreateSession,
    StartSignalling,
    SessionFull,
    SessionNotFound,
    SessionCancelled,

    Download,

    FileOfferResponse {
        answer: bool,
    },

    /// A new ICE candidate to be forwarded to the other participant
    #[serde(rename = "candidate")]
    NewIceCandidate {
        candidate: String,
        #[serde(rename = "sdpMid")]
        sdp_mid: String,
        #[serde(rename = "sdpMLineIndex")]
        sdp_mline_index: u32,
        #[serde(rename = "usernameFragment")]
        username_fragment: String,
    },

    /// A SDP offer that the creator of the session will send
    #[serde(rename = "offer")]
    SdpOffer(String),

    /// Answer from the other participant
    #[serde(rename = "answer")]
    SdpAnswer(String),
}

async fn websocket_signalling(
    ws: WebSocketUpgrade,
    Extension(data): Extension<Arc<AppData>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_websocket_signalling(socket, data))
}

async fn signalling_loop(
    mut socket: WebSocket,
    id: String,
    tx: broadcast::Sender<ClientMessage>,
    mut rx: broadcast::Receiver<ClientMessage>,
    mut cancellation: broadcast::Receiver<()>,
    is_uploader: bool,
) -> anyhow::Result<()> {
    let (mut ws_send, mut ws_recv) = socket.split();

    tokio::select! {
        res = async {
            loop {
                let msg = rx.recv().await?;

                debug!(id = ?id, uploader = ?is_uploader, "=> {msg:?}");

                ws_send.send(Message::Text(serde_json::to_string(&msg)?)).await?;
            }
        } => res,
        res = async {
            loop {
                let msg = ws_recv.next().await;


                match msg {
                    Some(Ok(Message::Text(txt))) => {
                        let msg = serde_json::from_str(&txt)?;

                        debug!(id = ?id, uploader = ?is_uploader, "<= {msg:?}");

                        tx.send(msg)?;
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
        res = cancellation.recv() => {
            ws_send.send(
                Message::Text(
                    serde_json::to_string(&ClientMessage::SessionCancelled).unwrap()
                )
            ).await?;

            Ok(())
        }
    }
}

async fn wait_for_message(socket: &mut WebSocket) -> anyhow::Result<ClientMessage> {
    loop {
        let msg = socket.recv().await;

        match msg {
            Some(Ok(Message::Text(txt))) => {
                dbg!(&txt);
                let msg = serde_json::from_str(&txt)?;

                return Ok(msg);
            }
            Some(Ok(Message::Close(close))) => {
                anyhow::bail!("WebSocket closed by remote: {close:?}")
            }
            Some(Err(e)) => Err(e)?,
            None => anyhow::bail!("No more message from WebSocket"),
            _ => {}
        }
    }
}
async fn handle_file_upload(
    mut socket: WebSocket,
    data: Arc<AppData>,
    file_name: String,
    size: u64,
) -> anyhow::Result<()> {
    let link = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect::<String>();

    debug!(id = ?link, "Got upload request");

    let (link, tx, rx, c_tx, c_rx) = {
        let mut sessions = data.sessions.write().await;
        if sessions.contains_key(&link) {
            anyhow::bail!("Already exists an uploader for link '{}'", link);
        }

        // TODO: don't hold RW lock for longer than necessary..
        socket
            .send(Message::Text(
                serde_json::to_string(&ClientMessage::UploadFileResponse { link: link.clone() })
                    .unwrap(),
            ))
            .await?;

        let state = SessionState::new(file_name, size);

        let cancellation_rx = state.cancellation_tx.subscribe();
        let cancellation_tx = state.cancellation_tx.clone();
        let (tx, rx) = state.get_sender().unwrap();

        sessions.insert(link.clone(), state);
        debug!("Created new session for '{link}'");

        (link, tx, rx, cancellation_tx, cancellation_rx)
    };

    match signalling_loop(socket, link.clone(), tx, rx, c_rx, true).await {
        Ok(_) => {
            debug!(id = ?link, uploader = true, "Finished signalling loop");
        },
        Err(e) => {
            error!(id = ?link, uploader = true, "Failed to finish signalling loop: {e}");
        },
    }

    debug!(id = ?link,"Removing session");
    let mut sessions = data.sessions.write().await;
    sessions.remove(&link);

    c_tx.send(())?;

    Ok(())
}

async fn handle_first_message(mut socket: WebSocket, data: Arc<AppData>) -> anyhow::Result<()> {
    match wait_for_message(&mut socket).await? {
        ClientMessage::UploadFile { name, size } => {
            handle_file_upload(socket, data, name, size).await?
        }
        ClientMessage::JoinSession { id } => handle_join_session(socket, data, id).await?,
        msg @ _ => anyhow::bail!("Unexpected message received: {:?}", msg),
    }

    Ok(())
}

async fn send(socket: &mut WebSocket, message: &ClientMessage) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(&message).unwrap()))
        .await?;

    Ok(())
}

async fn handle_join_session(
    mut socket: WebSocket,
    data: Arc<AppData>,
    id: String,
) -> anyhow::Result<()> {
    debug!(id = ?id, "Got join request");

    let (tx, rx, c_rx) = {
        let mut sessions = data.sessions.write().await;

        if let Some(state) = sessions.get_mut(&id) {
            if let Some((tx, rx)) = state.get_receiver() {
                send(
                    &mut socket,
                    &ClientMessage::SessionInfo {
                        file_name: state.file_name.clone(),
                        size: state.file_size,
                    },
                )
                .await?;

                (tx, rx, state.cancellation_tx.subscribe())
            } else {
                send(&mut socket, &ClientMessage::SessionFull).await?;

                anyhow::bail!("Session already full for '{}'", id);
            }
        } else {
            send(&mut socket, &ClientMessage::SessionNotFound).await?;

            anyhow::bail!("No session found for '{}'", id);
        }

        // TODO: don't hold RW lock for longer than necessary..
    };

    match signalling_loop(socket, id.clone(), tx, rx, c_rx, false).await {
        Ok(_) => {
            debug!(id = ?id, uploader = false, "Finished signalling loop");
        },
        Err(e) => {
            error!(id = ?id, uploader = false, "Failed to finish signalling loop: {e}");
        },
    }

    let mut sessions = data.sessions.write().await;
    if let Some(state) = sessions.get_mut(&id) {
        state.return_receiver();
    }

    Ok(())
}

async fn handle_websocket_signalling(socket: WebSocket, data: Arc<AppData>) -> Response<()> {
    match handle_first_message(socket, data).await {
        Ok(()) => Response::builder().status(StatusCode::OK).body(()).unwrap(),
        Err(e) => {
            error!("Failed to complete signalling: {e}");

            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(())
                .unwrap()
        }
    }
}

async fn start(addr: SocketAddr) -> anyhow::Result<()> {
    let data = Arc::new(AppData {
        sessions: Arc::new(RwLock::new(HashMap::new())),
    });

    let app = Router::new()
        .route("/signalling", get(websocket_signalling))
        .layer(AddExtensionLayer::new(data.clone()));

    info!("Listening for signalling attempts on {addr}");

    hyper::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

#[derive(argh::FromArgs)]
/// Command
struct Args {
    /// address to listen to
    #[argh(positional)]
    address: SocketAddr,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = argh::from_env();

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("debug"))
        .unwrap();

    let subscriber = FmtSubscriber::builder().with_env_filter(filter).finish();

    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { start(args.address).await })?;

    Ok(())
}
