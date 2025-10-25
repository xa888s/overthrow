use crate::tui::Message as UiMessage;
use crossterm::event::{Event, EventStream};
use futures::{FutureExt, SinkExt, StreamExt};
use overthrow_types::ClientMessage;
use serde_json::{from_str, to_string};
use tokio::{select, sync::mpsc, task};
use tokio_tungstenite as ws;
use ws::tungstenite::Message as WsMessage;

mod tui;
use tui::ui;
mod draw;

#[derive(Debug)]
pub enum Message {
    Server(ClientMessage),
    Term(Event),
    Quit,
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

fn needs_response(msg: &ClientMessage) -> bool {
    use ClientMessage as Msg;
    match msg {
        Msg::PlayerId(_) => false,
        Msg::GameId(_) => false,
        Msg::Info(_) => false,
        Msg::End(_) => false,
        Msg::GameCancelled => false,
        Msg::Outcome(_) => false,
        Msg::ActionChoices(_) => true,
        Msg::ChallengeChoice(_) => true,
        Msg::BlockChoices(_) => true,
        Msg::ReactionChoices(_) => true,
        Msg::VictimChoices(_) => true,
        Msg::OneFromThreeChoices(_) => true,
        Msg::TwoFromFourChoices(_) => true,
    }
}

#[tokio::main]
async fn main() {
    let (tui_sender, mut client_rx) = mpsc::channel(1);
    let (client_sender, tui_rx) = mpsc::channel(1);

    // spawn our TUI task
    let handle = task::spawn(async move { ui(tui_sender, tui_rx).await });

    let mut reader = EventStream::new();
    let (mut sender, mut receiver) = ws::connect_async("ws://localhost:3000/websocket")
        .await
        .unwrap()
        .0
        .split();
    'outer: loop {
        select! {
            Some(event) = reader.next() => {
                // forward crossterm events to TUI task
                let event = event.expect("Terminal shouldn't have errors").into();
                client_sender.send(event)
                    .await
                    .expect("Receiver should never be dropped");
            },
            Some(Ok(msg)) = receiver.next() => {
                // must be text, if not, then server is done
                let WsMessage::Text(msg) = msg else { break };
                let msg: ClientMessage = from_str(&msg).expect("Server always sends well formed responses");
                let needs_response = needs_response(&msg);

                // send ClientMessage to TUI task
                client_sender.send(msg.into())
                    .await
                    .expect("Receiver should never be dropped");

                if needs_response {
                    // receive ClientResponse from TUI task
                    let response = loop {
                        select! {
                            response = client_rx.recv() => break response,
                            Some(Ok(msg)) = receiver.next() => {
                                // server has shut down connection, so we can gracefully exit
                                if let WsMessage::Text(msg) = &msg
                                    && let Ok(ClientMessage::GameCancelled) = from_str(msg)
                                {
                                    eprintln!("Game has been cancelled, shutting down");
                                    client_sender.send(Message::Quit)
                                        .await
                                        .expect("TUI task shouldn't be shutdown");
                                    break 'outer;
                                }
                                eprintln!("Server sent message in between decisions: {:?}", msg)
                            },
                        }
                    };

                    let Some(UiMessage::Response(response)) = response else { break };

                    // send ClientResponse to server
                    let msg = WsMessage::text(to_string(&response).expect("Serialization should never fail"));
                    sender.send(msg).await.expect("Server should not close");
                }
            },
            Some(msg) = client_rx.recv() => {
                if let UiMessage::Quit = msg {
                    break
                } else if let UiMessage::Response(msg) = msg {
                    panic!("Received unexpected message from UI thread: {msg:?}")
                }
            }
            else => break
        }
    }

    // graceful shutdown of UI
    handle.await.expect("UI task shouldn't panic");

    // should close WebSocket gracefully
    receiver
        .reunite(sender)
        .expect("Should reunite")
        .close(None)
        .await
        .expect("Should close")
}
