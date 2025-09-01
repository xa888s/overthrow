use crate::{
    Disconnected,
    dispatcher::Senders,
    game::{BroadcastMessage, Choices, GameMessage, Info, PlayerChannel},
};

use super::AppState;
use axum::Error;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use futures::{
    SinkExt, StreamExt,
    stream::{SplitSink, SplitStream},
};
use overthrow_engine::{
    action::{Action, Block, Blocks, Challenge, Reaction},
    deck::Card,
    machine::{Outcome, Summary},
    players::PlayerId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::ops::ControlFlow;
use std::{net::SocketAddr, time::Duration};
use tokio::sync::mpsc::Sender;
use tokio::time::timeout;
use tokio::{select, sync::mpsc};
use tracing::instrument;

fn serialize<T: Serialize>(value: T) -> Utf8Bytes {
    serde_json::to_string(&value).unwrap().into()
}

fn deserialize<T: for<'a> Deserialize<'a>>(response: Utf8Bytes) -> Result<T, ClientError> {
    serde_json::from_str::<T>(response.as_str()).map_err(|_| ClientError::InvalidResponse)
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientMessage {
    PlayerId(PlayerId),
    Info(Info),
    End(Summary),
    GameCancelled,
    Outcome(Outcome),
    ChooseAction(Vec<Action>),
    ChooseChallenge(Challenge),
    ChooseBlock(Blocks),
    ChooseReaction(Vec<Reaction>),
    ChooseVictim([Card; 2]),
    ChooseOneFromThree([Card; 3]),
    ChooseTwoFromFour([Card; 4]),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientResponse {
    Block(Card),
    Challenge(bool),
    Action(Action),
    Reaction(Reaction),
    Victim(Card),
    ExchangeOne(Card),
    ExchangeTwo([Card; 2]),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientError {
    NotReady,
    InvalidResponse,
}

#[derive(Debug)]
enum Ending {
    Summary(Summary),
    GameCancelled,
}

#[derive(Debug)]
struct Context<'state> {
    client_sender: &'state mut SplitSink<WebSocket, Message>,
    client_receiver: &'state mut SplitStream<WebSocket>,
    senders: &'state mut Senders,
}

#[instrument(skip(stream, state))]
pub async fn client_handler(addr: SocketAddr, stream: WebSocket, state: AppState) {
    if client_handler_inner(addr, stream, state.register)
        .await
        .is_err()
    {
        tracing::error!(addr = %addr, "Player has disconnected");
        state
            .disconnected
            .send(Disconnected { addr })
            .await
            .unwrap();
    }
}

#[instrument(skip(stream, register))]
async fn client_handler_inner(
    addr: SocketAddr,
    stream: WebSocket,
    register: Sender<Sender<PlayerChannel>>,
) -> Result<(), Error> {
    // By splitting, we can send and receive at the same time.
    let (mut client_sender, mut client_receiver) = stream.split();

    // register client with dispatcher
    let (dispatch_sender, mut dispatch_receiver) = mpsc::channel(1);
    tracing::debug!("Registering new client with dispatcher");
    register.send(dispatch_sender).await.unwrap();

    // while we are waiting to connect to a game
    let (id, mut broadcast_receiver, (mut tx, mut rx)) = loop {
        select! {
            Some(game_channel) = dispatch_receiver.recv() => {
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

    // check for messages from the game itself, as there is nothing the player can do (yet)
    loop {
        select! {
            Some(Ok(Message::Text(message))) = client_receiver.next() => {
                tracing::debug!(player_id = ?id, "Received premature message from client: {message}");
                client_sender.send(Message::Text(serialize(ClientError::NotReady))).await?;
            },
            Some(message) = rx.recv() => {
                let context = Context {
                    client_receiver: &mut client_receiver,
                    client_sender: &mut client_sender,
                    senders: &mut tx,
                };

                handle_game_message(message, context).await?
            },
            Ok(broadcast) = broadcast_receiver.recv() => {
                let context = Context {
                    client_receiver: &mut client_receiver,
                    client_sender: &mut client_sender,
                    senders: &mut tx,
                };

                match handle_broadcast(broadcast, context).await? {
                    ControlFlow::Break(Ending::Summary(_summary)) => {
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

#[instrument(skip_all)]
async fn handle_broadcast(
    message: BroadcastMessage,
    context: Context<'_>,
) -> Result<ControlFlow<Ending>, Error> {
    match message {
        BroadcastMessage::End(summary) => {
            let message = Message::Text(serialize(ClientMessage::End(summary)));
            context.client_sender.send(message).await?;

            Ok(ControlFlow::Break(Ending::Summary(summary)))
        }
        BroadcastMessage::Info(info) => {
            let message = Message::Text(serialize(ClientMessage::Info(info)));
            context.client_sender.send(message).await?;

            Ok(ControlFlow::Continue(()))
        }

        BroadcastMessage::Outcome(outcome) => {
            let message = Message::Text(serialize(ClientMessage::Outcome(outcome)));
            context.client_sender.send(message).await?;

            Ok(ControlFlow::Continue(()))
        }
        BroadcastMessage::GameCancelled => {
            let message = Message::Text(serialize(ClientMessage::GameCancelled));
            context.client_sender.send(message).await?;

            Ok(ControlFlow::Break(Ending::GameCancelled))
        }
    }
}

async fn handle_game_message(message: GameMessage, context: Context<'_>) -> Result<(), Error> {
    match message {
        GameMessage::ChooseAction(choices) => handle_action_choices(choices, context).await,
        GameMessage::ChooseVictim(choices) => handle_choosing_victim(choices, context).await,
        GameMessage::ChooseOneFromThree(choices) => handle_choose_one(choices, context).await,
        GameMessage::ChooseTwoFromFour(choices) => handle_choose_two(choices, context).await,
    }
}

async fn handle_choose_two(choices: [Card; 4], context: Context<'_>) -> Result<(), Error> {
    let message = Message::Text(serialize(ClientMessage::ChooseTwoFromFour(choices)));
    context.client_sender.send(message).await?;

    // wait for client's choice
    loop {
        if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
            && let Ok(response) = deserialize(message)
            && let ClientResponse::ExchangeTwo(cards) = response
        {
            // TODO: verify cards are valid
            context.senders.choose_two.send(cards).await.unwrap();
            break Ok(());
        } else {
            send_invalid_response(context.client_sender).await?;
        }
    }
}

async fn send_invalid_response(sender: &mut SplitSink<WebSocket, Message>) -> Result<(), Error> {
    let err = Message::Text(serialize(ClientError::InvalidResponse));
    sender.send(err).await
}

// TODO: factor out some functions/macros to avoid so much repetition
async fn handle_choose_one(choices: [Card; 3], context: Context<'_>) -> Result<(), Error> {
    let message = Message::Text(serialize(ClientMessage::ChooseOneFromThree(choices)));
    context.client_sender.send(message).await?;

    // wait for client's choice
    loop {
        if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
            && let Ok(response) = deserialize(message)
            && let ClientResponse::ExchangeOne(card) = response
        {
            // TODO: verify card is valid
            context.senders.choose_one.send(card).await.unwrap();
            break Ok(());
        } else {
            send_invalid_response(context.client_sender).await?;
        }
    }
}

async fn handle_choosing_victim(choices: [Card; 2], context: Context<'_>) -> Result<(), Error> {
    let message = Message::Text(serialize(ClientMessage::ChooseVictim(choices)));
    context.client_sender.send(message).await?;

    // wait for client's choice
    loop {
        if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
            && let Ok(response) = deserialize(message)
            && let ClientResponse::Victim(card) = response
        {
            // TODO: verify card is valid
            context.senders.victim_card.send(card).await.unwrap();
            break Ok(());
        } else {
            send_invalid_response(context.client_sender).await?;
        }
    }
}

#[instrument(skip(context, choices))]
async fn handle_action_choices(choices: Choices, context: Context<'_>) -> Result<(), Error> {
    match choices {
        Choices::Actions(actions) => {
            let message = Message::Text(serialize(ClientMessage::ChooseAction(actions)));
            context.client_sender.send(message).await?;

            loop {
                if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
                    && let Ok(response) = deserialize(message)
                    && let ClientResponse::Action(action) = response
                {
                    // TODO: verify action is valid
                    context.senders.action.send(action).await.unwrap();
                    break Ok(());
                } else {
                    send_invalid_response(context.client_sender).await?;
                }
            }
        }
        Choices::Challenge(challenge) => {
            let message =
                Message::Text(serialize(ClientMessage::ChooseChallenge(challenge.clone())));
            context.client_sender.send(message).await?;

            loop {
                let response =
                    timeout(Duration::from_secs(10), context.client_receiver.next()).await;

                if response.is_err() {
                    // timeout has elapsed
                    break Ok(());
                } else if let Ok(Some(Ok(Message::Text(message)))) = response
                    && let Ok(response) = deserialize(message)
                    && let ClientResponse::Challenge(should_challenge) = response
                    && should_challenge
                {
                    context.senders.challenge.send(challenge).await.unwrap();
                    break Ok(());
                } else {
                    send_invalid_response(context.client_sender).await?;
                }
            }
        }
        Choices::Block(blocks) => {
            let message = Message::Text(serialize(ClientMessage::ChooseBlock(blocks.clone())));
            context.client_sender.send(message).await?;

            loop {
                let response =
                    timeout(Duration::from_secs(10), context.client_receiver.next()).await;
                if response.is_err() {
                    // timeout has elapsed
                    break Ok(());
                } else if let Ok(Some(Ok(Message::Text(message)))) = response
                    && let Ok(response) = deserialize(message)
                    && let ClientResponse::Block(block_as) = response
                {
                    handle_block(blocks, block_as, &context.senders.block).await;
                    break Ok(());
                } else {
                    send_invalid_response(context.client_sender).await?;
                }
            }
        }
        Choices::Reactions(reactions) => {
            let message =
                Message::Text(serialize(ClientMessage::ChooseReaction(reactions.clone())));
            context.client_sender.send(message).await?;

            loop {
                let response =
                    timeout(Duration::from_secs(10), context.client_receiver.next()).await;

                if response.is_err() {
                    // timeout has elapsed
                    break Ok(());
                } else if let Ok(Some(Ok(Message::Text(message)))) = response
                    && let Ok(response) = deserialize::<ClientResponse>(message)
                    && let ClientResponse::Reaction(reaction) = response
                {
                    // TODO: verify reaction is valid
                    match reaction {
                        Reaction::Block(block) => context.senders.block.send(block).await.unwrap(),
                        Reaction::Challenge(challenge) => {
                            context.senders.challenge.send(challenge).await.unwrap()
                        }
                    }
                    break Ok(());
                } else {
                    send_invalid_response(context.client_sender).await?;
                }
            }
        }
    }
}

async fn handle_block(blocks: Blocks, block_as: Card, sender: &Sender<Block>) {
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
    sender.send(block).await.unwrap();
}
