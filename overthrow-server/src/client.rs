use crate::{
    Disconnected,
    dispatcher::Senders,
    game::{BroadcastMessage, Choices, GameMessage, PlayerGameInfo},
};

use super::AppState;
use axum::Error;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use overthrow_engine::{
    action::{Blocks, Reaction},
    deck::Card,
    machine::Summary,
    match_to_indices,
};

use overthrow_types::*;

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::ops::ControlFlow;
use tokio::select;
use tokio::sync::{broadcast, oneshot};
use tracing::instrument;
use uuid::Uuid;

fn serialize<T: Serialize>(value: T) -> Utf8Bytes {
    serde_json::to_string(&value).unwrap().into()
}

fn deserialize<T: for<'a> Deserialize<'a>>(response: Utf8Bytes) -> Result<T, ClientError> {
    serde_json::from_str::<T>(response.as_str()).map_err(|_| ClientError::InvalidResponse)
}

#[derive(Debug)]
enum Ending {
    Summary(Summary),
    GameCancelled,
}

#[derive(Debug)]
struct ClientHandle<'state> {
    player_sender: &'state mut SplitSink<WebSocket, Message>,
    player_receiver: &'state mut SplitStream<WebSocket>,
    broadcast_receiver: &'state mut broadcast::Receiver<BroadcastMessage>,
    senders: &'state mut Senders,
}

impl<'state> ClientHandle<'state> {
    async fn send_to_client(&mut self, message: ClientMessage) -> Result<(), Error> {
        let message = Message::Text(serialize(message));
        self.player_sender.send(message).await
    }

    async fn send_game_cancelled(&mut self) -> Result<(), Error> {
        self.send_to_client(ClientMessage::GameCancelled).await
    }

    async fn send_not_ready(&mut self) -> Result<(), Error> {
        let not_ready = Message::Text(serialize(ClientError::NotReady));
        self.player_sender.send(not_ready).await
    }

    async fn send_invalid_response(&mut self) -> Result<(), Error> {
        let err = Message::Text(serialize(ClientError::InvalidResponse));
        self.player_sender.send(err).await
    }

    #[instrument(skip_all)]
    async fn handle_broadcast(
        &mut self,
        message: BroadcastMessage,
    ) -> Result<ControlFlow<Ending>, Error> {
        match message {
            BroadcastMessage::End(summary) => {
                self.send_to_client(ClientMessage::End(summary)).await?;
                Ok(ControlFlow::Break(Ending::Summary(summary)))
            }
            BroadcastMessage::Info(info) => {
                self.send_to_client(ClientMessage::Info(info)).await?;
                Ok(ControlFlow::Continue(()))
            }
            BroadcastMessage::Outcome(outcome) => {
                self.send_to_client(ClientMessage::Outcome(outcome)).await?;
                Ok(ControlFlow::Continue(()))
            }
            BroadcastMessage::GameCancelled => {
                self.send_to_client(ClientMessage::GameCancelled).await?;
                Ok(ControlFlow::Break(Ending::GameCancelled))
            }
            BroadcastMessage::ReactionTimeout => {
                panic!("Should never have a top level reaction timeout")
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
        self.send_to_client(ClientMessage::TwoFromFourChoices(choices))
            .await?;

        // matching found == chosen cards are valid
        let are_valid_choices = |cards| match_to_indices(cards, choices).is_some();

        // wait for client's choice
        loop {
            if let Some(Ok(Message::Text(message))) = self.player_receiver.next().await
                && let Ok(response) = deserialize(message)
                && let ClientResponse::ExchangeTwo(cards) = response
                && are_valid_choices(cards)
            {
                self.senders.choose_two.send(cards).await.unwrap();
                break Ok(());
            } else {
                self.send_invalid_response().await?;
            }
        }
    }

    async fn handle_choose_one(&mut self, choices: [Card; 3]) -> Result<(), Error> {
        self.send_to_client(ClientMessage::OneFromThreeChoices(choices))
            .await?;

        // wait for client's choice
        loop {
            if let Some(Ok(Message::Text(message))) = self.player_receiver.next().await
                && let Ok(response) = deserialize(message)
                && let ClientResponse::ExchangeOne(card) = response
                && choices.contains(&card)
            {
                self.senders.choose_one.send(card).await.unwrap();
                break Ok(());
            } else {
                self.send_invalid_response().await?;
            }
        }
    }

    async fn handle_choosing_victim(&mut self, choices: [Card; 2]) -> Result<(), Error> {
        self.send_to_client(ClientMessage::VictimChoices(choices))
            .await?;

        // wait for client's choice
        loop {
            if let Some(Ok(Message::Text(message))) = self.player_receiver.next().await
                && let Ok(response) = deserialize(message)
                && let ClientResponse::ChooseVictim(card) = response
                && choices.contains(&card)
            {
                self.senders.victim_card.send(card).await.unwrap();
                break Ok(());
            } else {
                self.send_invalid_response().await?;
            }
        }
    }

    #[instrument(skip(choices))]
    async fn handle_action_choices(&mut self, choices: Choices) -> Result<(), Error> {
        match choices {
            Choices::Actions(actions) => self.handle_actions(actions).await,
            Choices::Challenge(challenge) => self.handle_challenge(challenge).await,
            Choices::Block(blocks) => self.handle_blocks(blocks).await,
            Choices::Reactions(reactions) => self.handle_reactions(reactions).await,
        }
    }

    async fn handle_actions(&mut self, actions: Vec<Action>) -> Result<(), Error> {
        self.send_to_client(ClientMessage::ActionChoices(actions.clone()))
            .await?;

        loop {
            if let Some(Ok(Message::Text(message))) = self.player_receiver.next().await
                && let Ok(response) = deserialize(message)
                && let ClientResponse::Act(action) = response
                && actions.contains(&action)
            {
                self.senders.action.send(action).await.unwrap();
                break Ok(());
            } else {
                self.send_invalid_response().await?;
            }
        }
    }

    async fn handle_blocks(&mut self, blocks: Blocks) -> Result<(), Error> {
        self.send_to_client(ClientMessage::BlockChoices(blocks.clone()))
            .await?;

        loop {
            select! {
                Ok(BroadcastMessage::ReactionTimeout) = self.broadcast_receiver.recv() => break Ok(()),
                Some(Ok(Message::Text(msg))) = self.player_receiver.next() => {
                    // if parsing fails, send invalid response
                    let Ok(msg) = deserialize(msg) else { self.send_invalid_response().await?; continue };

                    // if pass do nothing, if block and valid block, send to game task, otherwise invalid response
                    match msg {
                        ClientResponse::Pass => break Ok(()),
                        ClientResponse::Block(block_as) if blocks.claims(block_as) => {
                            self.handle_block(blocks, block_as).await;
                            break Ok(());
                        }
                        _ => self.send_invalid_response().await?,
                    }
                }
            }
        }
    }

    async fn handle_challenge(&mut self, challenge: Challenge) -> Result<(), Error> {
        self.send_to_client(ClientMessage::ChallengeChoice(challenge.clone()))
            .await?;

        loop {
            select! {
                Ok(BroadcastMessage::ReactionTimeout) = self.broadcast_receiver.recv() => break Ok(()),
                Some(Ok(Message::Text(msg))) = self.player_receiver.next() => {
                    // if parsing fails, send invalid response
                    let Ok(msg) = deserialize::<ClientResponse>(msg) else { self.send_invalid_response().await?; continue };

                    // if pass do nothing, if challenge and valid challenge, send to game task, otherwise invalid response
                    match msg {
                        ClientResponse::Pass => break Ok(()),
                        ClientResponse::Challenge => {
                            self.senders.challenge.send(challenge).await.unwrap();
                            break Ok(());
                        },
                        _ => self.send_invalid_response().await?,
                    }
                }
            }
        }
    }

    async fn handle_reactions(&mut self, reactions: Vec<Reaction>) -> Result<(), Error> {
        self.send_to_client(ClientMessage::ReactionChoices(reactions.clone()))
            .await?;

        loop {
            select! {
                Ok(BroadcastMessage::ReactionTimeout) = self.broadcast_receiver.recv() => break Ok(()),
                Some(Ok(Message::Text(msg))) = self.player_receiver.next() => {
                    // if parsing fails, send invalid response
                    let Ok(msg) = deserialize::<ClientResponse>(msg) else { self.send_invalid_response().await?; continue };

                    // if pass do nothing, if react and valid reaction, send to game task, otherwise invalid response
                    match msg {
                        ClientResponse::Pass => break Ok(()),
                        ClientResponse::React(react) if reactions.contains(&react) => {
                            match react {
                                Reaction::Block(block) => self.senders.block.send(block).await.unwrap(),
                                Reaction::Challenge(challenge) => {
                                    self.senders.challenge.send(challenge).await.unwrap()
                                }
                            }

                            break Ok(());
                        }
                        _ => self.send_invalid_response().await?,
                    }
                }
            }
        }
    }

    async fn handle_block(&mut self, blocks: Blocks, block_as: Card) {
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
        self.senders.block.send(block).await.unwrap();
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
        (addr, game_id),
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
    (_, game_id): (SocketAddr, Uuid),
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
        channels: (mut tx, mut rx),
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
                return Err(Error::new("Client has left game"));
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
        broadcast_receiver: &mut broadcast_receiver.resubscribe(),
        senders: &mut tx,
    };

    // check for messages from the game itself, as there is nothing the player can do (yet)
    loop {
        select! {
            // TODO: find way to encapsulate player_receiver
            Some(Ok(message)) = client.player_receiver.next() => {
                if matches!(message, Message::Close(_)) {
                    break Err(Error::new("Client disconnected"));
                }
                let Message::Text(message) = message else { todo!() };
                tracing::debug!(player_id = ?id, "Received premature message from client: {message}");
                client.send_not_ready().await?;
            },
            Some(message) = rx.recv() => {
                select! {
                    res = client.handle_game_message(message) => res?,
                    Ok(BroadcastMessage::GameCancelled) = broadcast_receiver.recv() => {
                        client.send_game_cancelled().await?;
                        break Err(Error::new("Game was cancelled"));
                    },
                }
            },
            Ok(broadcast) = broadcast_receiver.recv() => {
                match client.handle_broadcast(broadcast).await? {
                    ControlFlow::Break(Ending::Summary(summary)) => {
                        break Ok(());
                    },
                    ControlFlow::Break(Ending::GameCancelled) => {
                        break Err(Error::new("Game was cancelled"));
                    },
                    _ => continue,
                }
            },
        }
    }
}
