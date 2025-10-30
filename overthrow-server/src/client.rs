use crate::{
    Disconnected,
    dispatcher::ClientChannels,
    game::{BroadcastMessage, Choices, GameMessage, Pass, PlayerGameInfo},
};

use super::AppState;
use axum::Error as AxumError;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use jiff::Timestamp;
use overthrow_engine::{
    action::{Blocks, Reaction},
    deck::Card,
    match_to_indices,
};
use thiserror::Error;

use overthrow_types::*;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::{net::SocketAddr, time::Duration};
use tokio::{select, time::timeout_at};
use tokio::{sync::oneshot, time::Instant};
use tracing::{debug, instrument, trace};
use uuid::Uuid;

fn serialize<T: Serialize>(value: T) -> Utf8Bytes {
    serde_json::to_string(&value).unwrap().into()
}

fn deserialize<T: for<'a> Deserialize<'a>>(response: &Utf8Bytes) -> Result<T, ClientError> {
    serde_json::from_str::<T>(response.as_str()).map_err(|_| ClientError::InvalidResponse)
}

#[derive(Debug, Error)]
enum Error {
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error("Game was cancelled")]
    GameCancelled,
    #[error("Client disconnected")]
    Disconnected,
}

// TODO: properly handle axum errors, for now we treat it as a disconnection
impl From<AxumError> for Error {
    fn from(_: AxumError) -> Self {
        Error::Disconnected
    }
}

#[derive(Debug)]
struct ClientHandle<'state> {
    player_sender: &'state mut SplitSink<WebSocket, Message>,
    player_receiver: &'state mut SplitStream<WebSocket>,
    senders: Arc<ClientChannels>,
}

impl<'state> ClientHandle<'state> {
    async fn send_to_client(&mut self, message: ClientMessage) -> Result<(), AxumError> {
        let message = Message::Text(serialize(message));
        self.player_sender.send(message).await
    }

    async fn send_game_cancelled(&mut self) -> Result<(), AxumError> {
        self.send_to_client(ClientMessage::GameCancelled).await
    }

    async fn send_not_ready(&mut self) -> Result<(), AxumError> {
        let not_ready = Message::Text(serialize(ClientError::NotReady));
        self.player_sender.send(not_ready).await
    }

    async fn send_invalid_response(&mut self) -> Result<(), AxumError> {
        let err = Message::Text(serialize(ClientError::InvalidResponse));
        self.player_sender.send(err).await
    }

    // tries to parse message and have response_handler process it. If the message provided by the client is invalid in some way,
    // the function will send the client an invalid response message and try again until it suceeds, the timeout completes
    // or the client disconnects
    async fn handle_timed_client_response<M, H>(
        &mut self,
        message_builder: M,
        mut response_handler: H,
    ) -> Result<(), Error>
    where
        M: FnOnce(Timestamp) -> ClientMessage,
        // unfortunately Arc is required because of a bug with AsyncFn(Mut) bounds
        H: AsyncFnMut(Arc<ClientChannels>, ClientResponse) -> Result<(), ClientError>,
    {
        // get our initial countdown time
        let (countdown_end, deadline) = get_countdown_time();
        trace!(countdown_end=%countdown_end, "Set countdown timeout");

        // send out initial message
        self.send_to_client(message_builder(countdown_end)).await?;

        loop {
            let result = match timeout_at(deadline, self.player_receiver.next()).await {
                Ok(message) => {
                    // if the client disconnects, the message is not a valid websocket message, it is not text,
                    // and/or it doesn't deserialize correctly, the response is invalid and we return Err
                    // Otherwise, we pass the response to the handler and see if it returns Ok
                    use ClientError as E;
                    let Ok(response) = message
                        .ok_or(Error::Disconnected)?
                        .and_then(|msg| msg.into_text())
                        .map_err(|_| E::InvalidResponse)
                        .and_then(|text| deserialize(&text))
                    else {
                        self.send_invalid_response().await?;
                        continue;
                    };

                    response_handler(Arc::clone(&self.senders), response).await
                }
                // timeout reached, send pass
                Err(_) => {
                    self.senders.pass.send(Pass).await.unwrap();
                    Ok(())
                }
            };

            // we only loop if the message is invalid
            if result.is_ok() {
                break Ok(());
            }
        }
    }

    // tries to parse message and have handler process it. If the message provided by the client is invalid in some way,
    // the function will send the client an invalid response message and try again until it suceeds or the client disconnects
    async fn handle_client_response<H>(
        &mut self,
        message: ClientMessage,
        mut response_handler: H,
    ) -> Result<(), Error>
    where
        // unfortunately Arc is required because of a bug with AsyncFn(Mut) bounds
        H: AsyncFnMut(Arc<ClientChannels>, ClientResponse) -> Result<(), ClientError>,
    {
        // send out initial message
        self.send_to_client(message).await?;

        loop {
            let message = self.player_receiver.next().await;

            // if the client disconnects, the message is not a valid websocket message, it is not text,
            // and/or it doesn't deserialize correctly, the response is invalid and we return
            // Otherwise, we pass the response to the handler and see if it returns
            use ClientError as E;

            let Ok(response) = message
                .ok_or(Error::Disconnected)?
                .and_then(|msg| msg.into_text())
                .map_err(|_| E::InvalidResponse)
                .and_then(|text| deserialize::<ClientResponse>(&text))
            else {
                self.send_invalid_response().await?;
                continue;
            };

            let result = response_handler(Arc::clone(&self.senders), response).await;

            // we only loop if the message is invalid
            if result.is_ok() {
                break Ok(());
            }
        }
    }

    async fn handle_game_message(&mut self, message: GameMessage) -> Result<(), Error> {
        match message {
            GameMessage::ChooseAction(choices) => self.handle_action_choices(choices).await,
            GameMessage::ChooseVictim(choices) => self.handle_choosing_victim(choices).await,
            GameMessage::ChooseOneFromThree(choices) => self.handle_choose_one(choices).await,
            GameMessage::ChooseTwoFromFour(choices) => self.handle_choose_two(choices).await,
        }
    }

    async fn handle_choose_two(&mut self, choices: [Card; 4]) -> Result<(), Error> {
        let message = ClientMessage::TwoFromFourChoices(choices);

        // matching found == chosen cards are valid
        let are_valid_choices = move |cards| match_to_indices(cards, choices).is_some();

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            if let ClientResponse::ExchangeTwo(cards) = msg
                && are_valid_choices(cards)
            {
                senders.choose_two.send(cards).await.unwrap();
                Ok(())
            } else {
                Err(ClientError::InvalidResponse)
            }
        };

        self.handle_client_response(message, response_handler).await
    }

    async fn handle_choose_one(&mut self, choices: [Card; 3]) -> Result<(), Error> {
        let message = ClientMessage::OneFromThreeChoices(choices);

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            if let ClientResponse::ExchangeOne(card) = msg
                && choices.contains(&card)
            {
                senders.choose_one.send(card).await.unwrap();
                Ok(())
            } else {
                Err(ClientError::InvalidResponse)
            }
        };

        self.handle_client_response(message, response_handler).await
    }

    async fn handle_choosing_victim(&mut self, choices: [Card; 2]) -> Result<(), Error> {
        let message = ClientMessage::VictimChoices(choices);

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            if let ClientResponse::ChooseVictim(card) = msg
                && choices.contains(&card)
            {
                senders.victim_card.send(card).await.unwrap();
                Ok(())
            } else {
                Err(ClientError::InvalidResponse)
            }
        };

        self.handle_client_response(message, response_handler).await
    }

    #[instrument(skip(self, choices))]
    async fn handle_action_choices(&mut self, choices: Choices) -> Result<(), Error> {
        match choices {
            Choices::Actions(actions) => self.handle_actions(actions).await,
            Choices::Challenge(challenge) => self.handle_challenge(challenge).await,
            Choices::Block(blocks) => self.handle_blocks(blocks).await,
            Choices::Reactions(reactions) => self.handle_reactions(reactions).await,
        }
    }

    async fn handle_actions(&mut self, actions: Vec<Action>) -> Result<(), Error> {
        let message = ClientMessage::ActionChoices(actions.clone());

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            if let ClientResponse::Act(action) = msg
                && actions.contains(&action)
            {
                senders.action.send(action).await.unwrap();
                Ok(())
            } else {
                Err(ClientError::InvalidResponse)
            }
        };

        self.handle_client_response(message, response_handler).await
    }

    async fn handle_blocks(&mut self, blocks: Blocks) -> Result<(), Error> {
        // FIXME: used to resolve higher-kinded lifetime errors
        let message_blocks = blocks.clone();
        let message_builder =
            move |timestamp| ClientMessage::BlockChoices(message_blocks, timestamp);

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            match msg {
                ClientResponse::Pass => senders.pass.send(Pass).await.unwrap(),
                ClientResponse::Block(block_as) if blocks.claims(block_as) => {
                    ClientHandle::handle_block(senders, blocks.clone(), block_as).await
                }
                _ => return Err(ClientError::InvalidResponse),
            }
            Ok(())
        };

        self.handle_timed_client_response(message_builder, response_handler)
            .await
    }

    async fn handle_challenge(&mut self, challenge: Challenge) -> Result<(), Error> {
        // FIXME: used to resolve higher-kinded lifetime errors
        let builder_challenge = challenge.clone();
        let builder =
            move |countdown_end| ClientMessage::ChallengeChoice(builder_challenge, countdown_end);

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            match msg {
                ClientResponse::Pass => senders.pass.send(Pass).await.unwrap(),
                ClientResponse::Challenge => {
                    senders.challenge.send(challenge.clone()).await.unwrap()
                }
                _ => return Err(ClientError::InvalidResponse),
            }
            Ok(())
        };

        self.handle_timed_client_response(builder, response_handler)
            .await
    }

    async fn handle_reactions(&mut self, reactions: Vec<Reaction>) -> Result<(), Error> {
        // FIXME: used to resolve higher-kinded lifetime errors
        let builder_reactions = reactions.clone();
        let message_builder =
            move |timestamp| ClientMessage::ReactionChoices(builder_reactions, timestamp);

        let response_handler = async move |senders: Arc<ClientChannels>, msg| {
            match msg {
                ClientResponse::Pass => senders.pass.send(Pass).await.unwrap(),
                ClientResponse::React(react) if reactions.contains(&react) => match react {
                    Reaction::Block(block) => senders.block.send(block).await.unwrap(),
                    Reaction::Challenge(challenge) => {
                        senders.challenge.send(challenge).await.unwrap()
                    }
                },
                _ => return Err(ClientError::InvalidResponse),
            }
            Ok(())
        };

        self.handle_timed_client_response(message_builder, response_handler)
            .await
    }

    async fn handle_block(senders: Arc<ClientChannels>, blocks: Blocks, block_as: Card) {
        let block = match blocks {
            Blocks::Other(block) => block,
            Blocks::Steal(b1, b2) => {
                if b1.claim() == block_as {
                    b1
                } else {
                    b2
                }
            }
        };
        senders.block.send(block).await.unwrap();
    }
}

#[instrument(skip(stream, state), fields(game_id))]
pub async fn client_handler(addr: SocketAddr, stream: WebSocket, state: AppState) {
    // By splitting, we can send and receive at the same time.
    let (mut client_sender, mut client_receiver) = stream.split();

    // register client with dispatcher
    let (dispatch_sender, dispatch_receiver) = oneshot::channel();
    let (game_id_sender, game_id_receiver) = oneshot::channel();
    tracing::debug!("Registering new client with dispatcher");
    state
        .register
        .send((dispatch_sender, game_id_sender))
        .await
        .expect("Should never fail to send to dispatcher");

    let game_id = game_id_receiver.await.expect("Should always send game id");
    // add game_id to context when logging
    tracing::Span::current().record("game_id", game_id.to_string());

    if client_handler_inner(
        game_id,
        dispatch_receiver,
        &mut client_sender,
        &mut client_receiver,
    )
    .await
    .is_err()
    {
        tracing::error!("Player has disconnected");
        state
            .disconnected
            .send(Disconnected { addr, game_id })
            .await
            .expect("Dispatcher should always be available");
    } else {
        // game is over and client is still connected, so we can close cleanly (ignore error if client disconnects right before sending this)
        let _ = client_receiver
            .reunite(client_sender)
            .expect("Should always reunite")
            .send(Message::Close(None))
            .await;
    }
}

async fn client_handler_inner(
    game_id: Uuid,
    mut dispatch_receiver: oneshot::Receiver<PlayerGameInfo>,
    client_sender: &mut SplitSink<WebSocket, Message>,
    client_receiver: &mut SplitStream<WebSocket>,
) -> Result<(), Error> {
    // seng game id first
    client_sender
        .send(Message::Text(serialize(ClientMessage::GameId(game_id))))
        .await?;

    // while we are waiting to connect to a game
    let PlayerGameInfo {
        id,
        mut broadcast_receiver,
        mut info,
        channels: (tx, mut rx),
    } = loop {
        select! {
            Ok(game_channel) = &mut dispatch_receiver => {
                // now a game has started, so we can break out of the loop
                break game_channel;
            }
            Some(Ok(message)) = client_receiver.next() => {
                if let Message::Text(text) = message {
                    tracing::debug!("Client sent data before game started: {text}");
                    let message = Message::Text(serialize(ClientError::NotReady));
                    client_sender.send(message).await?;
                }
            }
            else => {
                return Err(Error::Disconnected);
            }
        }
    };

    tracing::trace!(player_id = ?id, "Sending client their assigned PlayerId");
    // send client their assigned PlayerId
    client_sender
        .send(Message::Text(serialize(ClientMessage::PlayerId(id))))
        .await?;

    let mut client = ClientHandle {
        player_receiver: client_receiver,
        player_sender: client_sender,
        senders: Arc::new(tx),
    };

    // check for messages from the game itself, as there is nothing the player can do (yet)
    loop {
        select! {
            // TODO: find way to encapsulate player_receiver
            Some(Ok(message)) = client.player_receiver.next() => {
                if matches!(message, Message::Close(_)) {
                    break Err(Error::Disconnected);
                }

                debug!(player_id = ?id, "Received premature message from client: {message:?}");
                client.send_not_ready().await?;
            },
            // send client their views when we receive them
            Some(info) = info.recv() => client.send_to_client(ClientMessage::Info(info)).await?,
            Some(message) = rx.recv() => {
                select! {
                    res = client.handle_game_message(message) => res?,
                    Ok(BroadcastMessage::GameCancelled) = broadcast_receiver.recv() => {
                        client.send_game_cancelled().await?;
                        break Err(Error::GameCancelled);
                    },
                }
            },
            Ok(broadcast) = broadcast_receiver.recv() => {
                match broadcast {
                    BroadcastMessage::End(summary) => {
                        client.send_to_client(ClientMessage::End(summary)).await?;
                        todo!()
                    }
                    BroadcastMessage::Outcome(outcome) => client.send_to_client(ClientMessage::Outcome(outcome)).await?,
                    BroadcastMessage::GameCancelled => {
                        client.send_to_client(ClientMessage::GameCancelled).await?;
                        break Err(Error::GameCancelled);
                    }
                }
            },
        }
    }
}

fn get_countdown_time() -> (Timestamp, Instant) {
    // getting countdown timestamp
    let duration = Duration::from_secs(10);
    let countdown_end = Timestamp::now() + duration;
    let instant = Instant::now() + duration;

    (countdown_end, instant)
}
