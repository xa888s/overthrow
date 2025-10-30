use std::process::exit;

use clap::Parser;
use tokio::sync::mpsc::{self};
use tokio_tungstenite::tungstenite::http::Uri;

mod draw;
mod selector;
mod server;
mod tui;

#[derive(Debug, Parser)]
struct Args {
    #[arg(short, long)]
    address: Option<Uri>,

    #[arg(short, long, default_value_t = 3000)]
    port: u16,
}

#[tokio::main]
async fn main() {
    let (server_sender, local_receiver) = mpsc::channel(1);
    let (local_sender, server_receiver) = mpsc::channel(1);

    // console_subscriber::init();
    let args = Args::parse();
    let port = args.port;
    let host = args
        .address
        .as_ref()
        .and_then(|uri| uri.host())
        .unwrap_or("localhost");

    let address = Uri::builder()
        .scheme("ws")
        .authority(format!("{host}:{port}"))
        .path_and_query("/websocket")
        .build()
        .expect("Should be valid host and port");

    // start server handler
    let handle = tokio::spawn(async move {
        server::client_message_handler(address, server_sender, server_receiver).await
    });

    tui::ui(local_sender, local_receiver).await;

    // wait for server task to exit gracefully
    handle.await.unwrap().unwrap();
}
