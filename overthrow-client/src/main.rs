use crossterm::event::{Event, EventStream};
use futures::{FutureExt, SinkExt, StreamExt};
use overthrow_types::{ClientMessage, ClientResponse};
use serde_json::{from_str, to_string};
use tokio::{select, sync::mpsc, task};
use tokio_tungstenite as ws;
use ws::tungstenite::Message as WsMessage;

#[derive(Debug)]
enum Message {
    Server(ClientMessage),
    Term(Event),
}

impl From<ClientMessage> for Message {
    fn from(msg: ClientMessage) -> Self {
        Message::Server(msg)
    }
}

impl From<Event> for Message {
    fn from(event: Event) -> Self {
        Message::Term(event)
    }
}

#[tokio::main]
async fn main() {
    let (tui_sender, mut client_rx) = mpsc::channel(1);
    let (client_sender, tui_rx) = mpsc::channel(1);
    task::spawn_blocking(move || tui(tui_sender, tui_rx));
    let mut reader = EventStream::new();
    let (mut sender, mut receiver) = ws::connect_async("ws://localhost:3000/websocket")
        .await
        .unwrap()
        .0
        .split();

    loop {
        select! {
            Some(event) = reader.next().fuse() => {
                // forward crossterm events to TUI task
                let event = event.expect("Terminal shouldn't have errors").into();
                client_sender.send(event)
                    .await
                    .expect("Receiver should never be dropped");
            },
            Some(msg) = receiver.next().fuse() => {
                // deserialize into ClientMessage
                let msg = msg.expect("Message should never have errors");
                let msg = msg.to_text().expect("Should always be text");
                let msg: ClientMessage = from_str(msg).expect("Server always sends well formed responses");

                // send ClientMessage to TUI task
                client_sender.send(msg.into())
                    .await
                    .expect("Receiver should never be dropped");

                // receive ClientResponse from TUI task
                let response = client_rx.recv().await.expect("Should never be closed");

                // send ClientResponse to server
                let msg = WsMessage::text(to_string(&response).expect("Serialization should never fail"));
                sender.send(msg).await.expect("Server should not close");
            }
            else => break
        }
    }
}

#[derive(Debug)]
struct Context<'a> {
    pub sender: &'a mut mpsc::Sender<ClientResponse>,
    pub receiver: &'a mut mpsc::Receiver<Message>,
}

fn tui(mut sender: mpsc::Sender<ClientResponse>, mut receiver: mpsc::Receiver<Message>) {
    ratatui::init();

    loop {
        let Some(msg) = receiver.blocking_recv() else {
            break;
        };

        let context = Context {
            sender: &mut sender,
            receiver: &mut receiver,
        };

        match msg {
            Message::Server(msg) => handle_server_event(msg, context),
            Message::Term(event) => handle_term_event(event, context),
        }
    }

    ratatui::restore();
}

fn handle_server_event(msg: ClientMessage, ctx: Context) {
    todo!()
}

fn handle_term_event(event: Event, ctx: Context) {
    todo!()
}
