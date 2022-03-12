use futures_util::{stream::StreamExt, SinkExt};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::sync::RwLock;
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};
use tracing::*;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use std::collections::HashMap;
use std::sync::Mutex;
use std::{net::SocketAddr, sync::Arc};

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

#[derive(Serialize, Deserialize, Debug, Clone)]
enum ClientMessage {
    /// An offer to upload a file
    #[serde(rename = "upload")]
    UploadFile { name: String, size: u64 },

    /// Response to file upload
    #[serde(rename = "response")]
    UploadFileResponse { link: String },

    /// Request to join a session
    #[serde(rename = "join")]
    JoinSession { id: String },

    #[serde(rename = "signal")]
    StartSignalling,

    #[serde(rename = "full")]
    SessionFull,

    #[serde(rename = "notfound")]
    SessionNotFound,

    #[serde(rename = "cancelled")]
    SessionCancelled,

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

    /// Information about a session
    #[serde(rename = "info")]
    SessionInfo { file_name: String, size: u64 },

    /// Someone has joined the session
    #[serde(rename = "participant")]
    ParticipantJoin,

    /// Someone has left the session
    #[serde(rename = "participantleft")]
    ParticipantLeave,
}

async fn signalling_loop(
    socket: WebSocketStream<TcpStream>,
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

async fn wait_for_message(
    socket: &mut WebSocketStream<TcpStream>,
) -> anyhow::Result<ClientMessage> {
    loop {
        let msg = socket.next().await;

        match msg {
            Some(Ok(Message::Text(txt))) => {
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
    mut socket: WebSocketStream<TcpStream>,
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
        }
        Err(e) => {
            error!(id = ?link, uploader = true, "Failed to finish signalling loop: {e}");
        }
    }

    debug!(id = ?link,"Removing session");
    let mut sessions = data.sessions.write().await;
    sessions.remove(&link);

    c_tx.send(())?;

    Ok(())
}

async fn handle_first_message(
    mut socket: WebSocketStream<TcpStream>,
    data: Arc<AppData>,
) -> anyhow::Result<()> {
    match wait_for_message(&mut socket).await? {
        ClientMessage::UploadFile { name, size } => {
            handle_file_upload(socket, data, name, size).await?
        }
        ClientMessage::JoinSession { id } => handle_join_session(socket, data, id).await?,
        msg @ _ => anyhow::bail!("Unexpected message received: {:?}", msg),
    }

    Ok(())
}

async fn send(
    socket: &mut WebSocketStream<TcpStream>,
    message: &ClientMessage,
) -> anyhow::Result<()> {
    socket
        .send(Message::Text(serde_json::to_string(&message).unwrap()))
        .await?;

    Ok(())
}

async fn handle_join_session(
    mut socket: WebSocketStream<TcpStream>,
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
        }
        Err(e) => {
            error!(id = ?id, uploader = false, "Failed to finish signalling loop: {e}");
        }
    }

    let mut sessions = data.sessions.write().await;
    if let Some(state) = sessions.get_mut(&id) {
        state.return_receiver();
    }

    Ok(())
}

async fn handle_connection(
    data: Arc<AppData>,
    raw_stream: TcpStream,
    addr: SocketAddr,
) -> anyhow::Result<()> {
    debug!("Incoming TCP connection from: {}", addr);

    let ws_stream = tokio_tungstenite::accept_async(raw_stream).await?;

    debug!("WebSocket connection established: {}", addr);

    if let Err(e) = handle_first_message(ws_stream, data).await {
        warn!("Failed to do signalling: {e}");
    }

    Ok(())
}

async fn start(addr: SocketAddr) -> anyhow::Result<()> {
    let data = Arc::new(AppData {
        sessions: Arc::new(RwLock::new(HashMap::new())),
    });

    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    info!("Listening for WebSocket connections on: {}", addr);

    while let Ok((stream, addr)) = listener.accept().await {
        tokio::spawn(handle_connection(data.clone(), stream, addr));
    }

    Ok(())
}

const HELP: &str = "\
yeet signalling server

USAGE:
  yeet-signal-server address

FLAGS:
  -h, --help            Prints help information

ARGS:
  address               The address to listen to
";

struct Args {
    address: SocketAddr,
}

fn parse_args() -> Result<Args, pico_args::Error> {
    let mut pargs = pico_args::Arguments::from_env();

    if pargs.contains(["-h", "--help"]) {
        print!("{}", HELP);
        std::process::exit(0);
    }

    let args = Args {
        address: pargs.free_from_str()?,
    };

    Ok(args)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Args = match parse_args() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error: {}.", e);
            std::process::exit(1);
        }
    };

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
