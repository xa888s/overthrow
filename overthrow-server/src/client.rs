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
    action::{Block, Blocks, Reaction},
    deck::Card,
    machine::Summary,
    match_to_indices,
};

use overthrow_types::*;

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::ops::ControlFlow;
use tokio::select;
use tokio::sync::{broadcast, mpsc::Sender, oneshot};
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
    client_sender: &'state mut SplitSink<WebSocket, Message>,
    client_receiver: &'state mut SplitStream<WebSocket>,
    broadcast_receiver: &'state mut broadcast::Receiver<BroadcastMessage>,
    senders: &'state mut Senders,
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
        tracing::error!(addr = %addr, "Player has disconnected");
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
    (addr, game_id): (SocketAddr, Uuid),
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
                } else if let Message::Close(_) = message {
                    tracing::error!(addr = %addr, "Client sent close frame while waiting for game");
                    return Err(Error::new("Client has left game"))
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

    let context = ClientHandle {
        client_receiver,
        client_sender,
        broadcast_receiver: &mut broadcast_receiver.resubscribe(),
        senders: &mut tx,
    };

    // check for messages from the game itself, as there is nothing the player can do (yet)
    loop {
        select! {
            Some(Ok(message)) = client_receiver.next() => {
                if matches!(message, Message::Close(_)) {
                    break Err(Error::new("Client disconnected"));
                }
                let Message::Text(message) = message else { todo!() };
                tracing::debug!(player_id = ?id, "Received premature message from client: {message}");
                client_sender.send(Message::Text(serialize(ClientError::NotReady))).await?;
            },
            Some(message) = rx.recv() => {
                let context = ClientHandle {
                    client_receiver,
                    client_sender,
                    broadcast_receiver: &mut broadcast_receiver.resubscribe(),
                    senders: &mut tx,
                };

                select! {
                    // res = handle_game_message(message, context) => res?,
                    Ok(BroadcastMessage::GameCancelled) = broadcast_receiver.recv() => {
                        client_sender
                            .send(Message::Text(serialize(ClientMessage::GameCancelled)))
                            .await?;
                        break Err(Error::new("Game was cancelled"));
                    },
                }
            },
            Ok(broadcast) = broadcast_receiver.recv() => {
                let context = ClientHandle {
                    client_receiver,
                    client_sender,
                    broadcast_receiver: &mut broadcast_receiver.resubscribe(),
                    senders: &mut tx,
                };

                match handle_broadcast(broadcast, context).await? {
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

async fn send_to_client(
    message: ClientMessage,
    sender: &mut SplitSink<WebSocket, Message>,
) -> Result<(), Error> {
    let message = Message::Text(serialize(message));
    sender.send(message).await
}

#[instrument(skip_all)]
async fn handle_broadcast(
    message: BroadcastMessage,
    ctx: ClientHandle<'_>,
) -> Result<ControlFlow<Ending>, Error> {
    match message {
        BroadcastMessage::End(summary) => {
            send_to_client(ClientMessage::End(summary), ctx.client_sender).await?;
            Ok(ControlFlow::Break(Ending::Summary(summary)))
        }
        BroadcastMessage::Info(info) => {
            send_to_client(ClientMessage::Info(info), ctx.client_sender).await?;
            Ok(ControlFlow::Continue(()))
        }
        BroadcastMessage::Outcome(outcome) => {
            send_to_client(ClientMessage::Outcome(outcome), ctx.client_sender).await?;
            Ok(ControlFlow::Continue(()))
        }
        BroadcastMessage::GameCancelled => {
            send_to_client(ClientMessage::GameCancelled, ctx.client_sender).await?;
            Ok(ControlFlow::Break(Ending::GameCancelled))
        }
        BroadcastMessage::ReactionTimeout => {
            panic!("Should never have a top level reaction timeout")
        }
    }
}

async fn handle_game_message(message: GameMessage, context: ClientHandle<'_>) -> Result<(), Error> {
    match message {
        GameMessage::ChooseAction(choices) => handle_action_choices(choices, context).await,
        GameMessage::ChooseVictim(choices) => handle_choosing_victim(choices, context).await,
        GameMessage::ChooseOneFromThree(choices) => handle_choose_one(choices, context).await,
        GameMessage::ChooseTwoFromFour(choices) => handle_choose_two(choices, context).await,
    }
}

async fn handle_choose_two(choices: [Card; 4], context: ClientHandle<'_>) -> Result<(), Error> {
    let message = Message::Text(serialize(ClientMessage::TwoFromFourChoices(choices)));
    context.client_sender.send(message).await?;

    // matching found == chosen cards are valid
    let are_valid_choices = |cards| match_to_indices(cards, choices).is_some();

    // wait for client's choice
    loop {
        if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
            && let Ok(response) = deserialize(message)
            && let ClientResponse::ExchangeTwo(cards) = response
            && are_valid_choices(cards)
        {
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
async fn handle_choose_one(choices: [Card; 3], context: ClientHandle<'_>) -> Result<(), Error> {
    let message = Message::Text(serialize(ClientMessage::OneFromThreeChoices(choices)));
    context.client_sender.send(message).await?;

    // wait for client's choice
    loop {
        if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
            && let Ok(response) = deserialize(message)
            && let ClientResponse::ExchangeOne(card) = response
            && choices.contains(&card)
        {
            context.senders.choose_one.send(card).await.unwrap();
            break Ok(());
        } else {
            send_invalid_response(context.client_sender).await?;
        }
    }
}

async fn handle_choosing_victim(
    choices: [Card; 2],
    context: ClientHandle<'_>,
) -> Result<(), Error> {
    let message = Message::Text(serialize(ClientMessage::VictimChoices(choices)));
    context.client_sender.send(message).await?;

    // wait for client's choice
    loop {
        if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
            && let Ok(response) = deserialize(message)
            && let ClientResponse::ChooseVictim(card) = response
            && choices.contains(&card)
        {
            context.senders.victim_card.send(card).await.unwrap();
            break Ok(());
        } else {
            send_invalid_response(context.client_sender).await?;
        }
    }
}

#[instrument(skip(context, choices))]
async fn handle_action_choices(choices: Choices, context: ClientHandle<'_>) -> Result<(), Error> {
    match choices {
        Choices::Actions(actions) => {
            let message = Message::Text(serialize(ClientMessage::ActionChoices(actions.clone())));
            context.client_sender.send(message).await?;

            loop {
                if let Some(Ok(Message::Text(message))) = context.client_receiver.next().await
                    && let Ok(response) = deserialize(message)
                    && let ClientResponse::Act(action) = response
                    && actions.contains(&action)
                {
                    context.senders.action.send(action).await.unwrap();
                    break Ok(());
                } else {
                    send_invalid_response(context.client_sender).await?;
                }
            }
        }
        Choices::Challenge(challenge) => {
            let message =
                Message::Text(serialize(ClientMessage::ChallengeChoice(challenge.clone())));
            context.client_sender.send(message).await?;

            loop {
                select! {
                    Ok(BroadcastMessage::ReactionTimeout) = context.broadcast_receiver.recv() => break Ok(()),
                    Some(Ok(Message::Text(msg))) = context.client_receiver.next() => {
                        if let Ok(response) = deserialize(msg)
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
            }
        }
        Choices::Block(blocks) => {
            let message = Message::Text(serialize(ClientMessage::BlockChoices(blocks.clone())));
            context.client_sender.send(message).await?;

            loop {
                select! {
                    Ok(BroadcastMessage::ReactionTimeout) = context.broadcast_receiver.recv() => break Ok(()),
                    Some(Ok(Message::Text(msg))) = context.client_receiver.next() => {
                        if let Ok(response) = deserialize(msg)
                            && let ClientResponse::Block(block_as) = response
                            && blocks.claims(block_as)
                        {
                            handle_block(blocks, block_as, &context.senders.block).await;
                            break Ok(());
                        } else {
                            send_invalid_response(context.client_sender).await?;
                        }
                    }
                }
            }
        }
        Choices::Reactions(reactions) => {
            let message =
                Message::Text(serialize(ClientMessage::ReactionChoices(reactions.clone())));
            context.client_sender.send(message).await?;

            loop {
                select! {
                    Ok(BroadcastMessage::ReactionTimeout) = context.broadcast_receiver.recv() => break Ok(()),
                    Some(Ok(Message::Text(msg))) = context.client_receiver.next() => {
                        if let Ok(response) = deserialize(msg)
                            && let ClientResponse::React(reaction) = response
                            && reactions.contains(&reaction)
                        {
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
