use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures::{sink::SinkExt, stream::StreamExt};
use notify::{RecursiveMode, Result};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::Path;
use std::sync::mpsc::channel;
use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::broadcast::{self, Sender};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

// Our shared state
struct AppState {
    user_set: Mutex<HashSet<String>>,
    tx: broadcast::Sender<String>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "example_chat=trace".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let user_set = Mutex::new(HashSet::new());
    let (tx, _rx) = broadcast::channel(100);

    let tx2 = tx.clone();
    tokio::spawn(async move {
        tracing::info!("hello from thread, ready to listen to file changes :)");
        static FILE_NAME: &str = "C:/Users/tobia/OneDrive/Desktop/db.json";
        let path = Path::new(FILE_NAME);

        if let Err(e) = watch(path, tx2) {
            println!("error: {:?}", e)
        }
    });

    let app_state = Arc::new(AppState { user_set, tx });

    let app = Router::new()
        .route("/", get(index))
        .route("/websocket", get(websocket_handler))
        .with_state(app_state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| websocket(socket, state))
}

async fn websocket(stream: WebSocket, state: Arc<AppState>) {
    // By splitting we can send and receive at the same time.
    let (mut sender, mut receiver) = stream.split();

    // Username gets set in the receive loop, if it's valid.
    let mut username = String::new();
    // Loop until a text message is found.
    while let Some(Ok(message)) = receiver.next().await {
        if let Message::Text(name) = message {
            // If username that is sent by client is not taken, fill username string.
            check_username(&state, &mut username, &name);

            // If not empty we want to quit the loop else we want to quit function.
            if !username.is_empty() {
                break;
            } else {
                // Only send our client that username is taken.
                let _ = sender
                    .send(Message::Text(String::from("Username already taken.")))
                    .await;

                return;
            }
        }
    }

    // Subscribe before sending joined message.
    let mut rx = state.tx.subscribe();

    // Send joined message to all subscribers.
    let msg = format!("{} joined.", username);
    tracing::debug!("{}", msg);
    let _ = state.tx.send(msg);

    // This task will receive broadcast messages and send text message to our client.
    let mut send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            // In any websocket error, break loop.
            match sender.send(Message::Text(msg)).await {
                Ok(msg) => {
                    tracing::info!("Everything ok: {:?}", msg)
                }
                Err(e) => {
                    tracing::error!("Not ok: {}", e);
                    break;
                }
            }
        }
    });

    // Clone things we want to pass to the receiving task.
    let tx = state.tx.clone();
    let name = username.clone();

    // This task will receive `websocket` messages from client and send them to broadcast subscribers.
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(Message::Text(text))) = receiver.next().await {
            // Add username before message.
            match tx.send(format!("{}: {}", name, text)) {
                Ok(msg) => {
                    tracing::info!("tx -> Everything ok from {}: {:?}", name, msg)
                }
                Err(e) => {
                    tracing::error!("tx -> Not ok: {}", e);
                    break;
                }
            }
        }
    });

    // If any one of the tasks exit, abort the other.
    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    };

    // Send user left message.
    let msg = format!("{} left.", username);
    tracing::debug!("{}", msg);
    let _ = state.tx.send(msg);
    // Remove username from map so new clients can take it.
    state.user_set.lock().unwrap().remove(&username);
}

fn check_username(state: &AppState, string: &mut String, name: &str) {
    let mut user_set = state.user_set.lock().unwrap();

    if !user_set.contains(name) {
        user_set.insert(name.to_owned());

        string.push_str(name);
    }
}

// Include utf-8 file at **compile** time.
async fn index() -> Html<&'static str> {
    Html(std::include_str!("../chat.html"))
}

fn watch<P: AsRef<Path>>(path: P, tx: Sender<String>) -> Result<()> {
    let (tx1, rx1) = channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    // let mut watcher = RecommendedWatcher::new(tx, Config::default().with_compare_contents(true))?;
    let mut debouncer = new_debouncer(Duration::from_secs(1), Some(Duration::from_secs(1)), tx1)?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    // watcher.watch(path.as_ref(), RecursiveMode::NonRecursive)?;
    debouncer
        .watcher()
        .watch(path.as_ref(), RecursiveMode::NonRecursive)?;

    for res in rx1 {
        match res {
            // If there is a match execute the logevent function with the event::notify::Event as input
            // TODO: Only check for event.kind == Any (not AnyContinous because these are fired multiple times)
            Ok(event) => {
                if event[0].kind == DebouncedEventKind::Any {
                    tracing::info!("changed: {:?}", event);
                    if let Err(e) = tx.send(String::from(format!(
                        "File change detected: {:?}",
                        event[0].path
                    ))) {
                        tracing::error!("Got a tx sending error: {}", e)
                    }
                }
            }
            Err(e) => tracing::error!("watch error: {:?}", e),
        }
    }

    Ok(())
}
