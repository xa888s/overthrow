use futures::SinkExt;
use futures::StreamExt;
use overthrow_types::{ClientMessage, ClientResponse};
use serde_json::from_str;
use serde_json::to_string;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender},
};
use tokio_tungstenite as ws;
use tokio_tungstenite::tungstenite::http::Uri;
use ws::tungstenite::Error;
use ws::tungstenite::Message as WsMessage;

#[derive(Debug)]
pub enum LocalMessage {
    Response(ClientResponse),
    Quit,
}

fn wrap_message(msg: ClientResponse) -> WsMessage {
    WsMessage::text(to_string(&msg).expect("Serialization should never fail"))
}

// this basically just acts as a relay to the server so the main task doesn't get stuck
pub async fn client_message_handler(
    address: Uri,
    local_sender: Sender<ClientMessage>,
    mut local_receiver: Receiver<LocalMessage>,
) -> Result<(), Error> {
    let (mut server_sender, mut server_receiver) = ws::connect_async(address).await?.0.split();

    loop {
        select! {
            Some(Ok(msg)) = server_receiver.next() => {
                // must be text, if not, then server is done
                let WsMessage::Text(msg) = msg else { todo!() };
                let msg: ClientMessage = from_str(&msg).expect("Server always sends well formed responses");

                // send ClientMessage to main task
                let Ok(()) = local_sender.send(msg).await else { break };
            },
            Some(msg) = local_receiver.recv() => {
                match msg {
                    LocalMessage::Response(res) => {
                        // send ClientResponse to server
                        let Ok(()) = server_sender.send(wrap_message(res)).await else { break };
                    },
                    LocalMessage::Quit => break,
                }
            },
            else => todo!(),
        }
    }

    // should close WebSocket gracefully
    server_receiver
        .reunite(server_sender)
        .expect("Should reunite")
        .close(None)
        .await
        .expect("Should close");

    Ok(())
}
