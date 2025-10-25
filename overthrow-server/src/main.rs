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
use clap::Parser;
use client::client_handler;
use dispatcher::dispatcher;
use overthrow_types::{ClientError, ClientMessage, ClientResponse};
use schemars::schema_for;
use std::{fs, net::SocketAddr};
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};
use tracing_subscriber::{
    EnvFilter, Layer, layer::SubscriberExt, registry::LookupSpan, util::SubscriberInitExt,
};
use uuid::Uuid;

use crate::game::PlayerGameInfo;

#[derive(Clone, Debug)]
pub struct Disconnected {
    addr: SocketAddr,
    game_id: Uuid,
}

#[derive(Clone, Debug)]
struct AppState {
    // for registering a task/connection with the dispatcher
    register: Sender<(oneshot::Sender<PlayerGameInfo>, oneshot::Sender<Uuid>)>,
    disconnected: Sender<Disconnected>,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    generate: bool,

    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    // used to write out the schemas used for communicating with clients
    if args.generate {
        generate_schemas();
        return;
    }

    // attach logger for terminal and tokio-console
    tracing_subscriber::registry()
        .with(console_subscriber::spawn())
        .with(
            tracing_subscriber::fmt::layer().with_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| format!("{}=trace", env!("CARGO_CRATE_NAME")).into()),
            ),
        )
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

    // listen on all ports
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", args.port))
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

fn generate_schemas() {
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
}
