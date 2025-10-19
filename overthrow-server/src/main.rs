mod client;
mod dispatcher;
mod game;

use axum::{
    Extension, Router,
    extract::{ConnectInfo, ws::WebSocketUpgrade},
    handler::Handler,
    response::{Html, IntoResponse},
    routing::get,
};
use client::client_handler;
use dispatcher::dispatcher;
use game::PlayerChannel;
use overthrow_types::{ClientError, ClientMessage, ClientResponse};
use schemars::schema_for;
use std::{fs, net::SocketAddr};
use tokio::sync::mpsc::{self, Sender};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone, Debug)]
pub struct Disconnected {
    addr: SocketAddr,
}

#[derive(Clone, Debug)]
struct AppState {
    // for registering a task/connection with the dispatcher
    register: Sender<Sender<PlayerChannel>>,
    disconnected: Sender<Disconnected>,
}

#[tokio::main]
async fn main() {
    // used to write out the schemas used for communicating with clients
    if let Some(arg) = std::env::args().nth(1)
        && arg == "generate"
    {
        let schema = schema_for!(ClientMessage);
        fs::write(
            "./client_message.json",
            serde_json::to_string_pretty(&schema).unwrap(),
        )
        .unwrap();

        let schema = schema_for!(ClientResponse);

        fs::write(
            "./client_response.json",
            serde_json::to_string_pretty(&schema).unwrap(),
        )
        .unwrap();

        let schema = schema_for!(ClientError);

        fs::write(
            "./client_error.json",
            serde_json::to_string_pretty(&schema).unwrap(),
        )
        .unwrap();

        return;
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| format!("{}=trace", env!("CARGO_CRATE_NAME")).into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // create channel for connections to register with dispatcher
    let (register, receiver) = mpsc::channel(10);
    let (disconnected_tx, disconnected_rx) = mpsc::channel(10);
    tokio::spawn(dispatcher(receiver, disconnected_rx));

    let app_state = AppState {
        register,
        disconnected: disconnected_tx,
    };

    let websocket_handler = websocket_handler.layer(Extension(app_state));

    let app = Router::new()
        .route("/", get(index))
        .route("/websocket", get(websocket_handler));

    let port = 3000;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    tracing::debug!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn websocket_handler(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    ws: WebSocketUpgrade,
    Extension(state): Extension<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| client_handler(addr, socket, state))
}

// Include utf-8 file at **compile** time.
async fn index() -> Html<&'static str> {
    Html(std::include_str!("../client.html"))
}
