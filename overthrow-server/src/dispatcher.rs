use crate::Disconnected;
use crate::game::BroadcastMessage;

use super::game::GameMessage;

use super::game::{PlayerChannel, coup_game};
use overthrow_engine::action::{Action, Block, Challenge};
use overthrow_engine::deck::Card;
use overthrow_engine::players::PlayerId;
use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::instrument;

pub type PlayerHalf = (Senders, Receiver<GameMessage>);
pub type GameHalf = (Sender<GameMessage>, Receivers);
pub type TaskReceiver = Receiver<Sender<PlayerChannel>>;
type Channels = (Vec<PlayerChannel>, HashMap<PlayerId, GameHalf>);

// Each client has 6 senders and 1 receiver:
// The receiver receives GameMessages, while the senders are for different types of choices (Action, Challenge, choosing, etc.)
#[derive(Debug)]
pub struct Senders {
    pub action: Sender<Action>,
    pub challenge: Sender<Challenge>,
    pub block: Sender<Block>,
    pub victim_card: Sender<Card>,
    pub choose_one: Sender<Card>,
    pub choose_two: Sender<[Card; 2]>,
}

// The game task has 6 receivers and 1 sender per client
// The sender is for GameMessages, and the receivers are for receiving game messages depending on which type it is
#[derive(Debug)]
pub struct Receivers {
    pub action: Receiver<Action>,
    pub challenge: Receiver<Challenge>,
    pub block: Receiver<Block>,
    pub victim_card: Receiver<Card>,
    pub choose_one: Receiver<Card>,
    pub choose_two: Receiver<[Card; 2]>,
}

fn generate_channels(
    len: usize,
    broadcaster: Arc<broadcast::Sender<BroadcastMessage>>,
) -> Channels {
    // broadcast channel for general updates
    PlayerId::iter()
        .take(len)
        .map(|id| {
            // create channels for sending messages between players and the game task
            let (game_tx, player_rx) = mpsc::channel(1);
            let (action_tx, action_rx) = mpsc::channel(1);
            let (challenge_tx, challenge_rx) = mpsc::channel(1);
            let (block_tx, block_rx) = mpsc::channel(1);
            let (victim_tx, victim_rx) = mpsc::channel(1);
            let (choose_one_tx, choose_one_rx) = mpsc::channel(1);
            let (choose_two_tx, choose_two_rx) = mpsc::channel(1);

            let senders = Senders {
                action: action_tx,
                challenge: challenge_tx,
                block: block_tx,
                victim_card: victim_tx,
                choose_one: choose_one_tx,
                choose_two: choose_two_tx,
            };

            let receivers = Receivers {
                action: action_rx,
                challenge: challenge_rx,
                block: block_rx,
                victim_card: victim_rx,
                choose_one: choose_one_rx,
                choose_two: choose_two_rx,
            };

            let player_half = (id, broadcaster.subscribe(), (senders, player_rx));
            let game_half = (id, (game_tx, receivers));
            (player_half, game_half)
        })
        .collect()
}

#[instrument(skip(task_receiver, disconnected))]
pub async fn dispatcher(mut task_receiver: TaskReceiver, mut disconnected: Receiver<Disconnected>) {
    let mut connections: Vec<Sender<PlayerChannel>> = Vec::new();
    let broadcaster = Arc::new(broadcast::channel::<BroadcastMessage>(2).0);
    loop {
        select! {
            Some(sender) = task_receiver.recv() => {
                connections.push(sender);
                if connections.len() >= 2 {
                    tracing::debug!("Sufficient players joined, starting game");
                    let connections = mem::take(&mut connections);

                    let (player_half, game_half) = generate_channels(connections.len(), broadcaster.clone());

                    // start the game task to run in the background
                    tracing::trace!("Starting coup game task");
                    tokio::spawn(coup_game(game_half, broadcaster.clone()));

                    // send back the player task's half of the channel, so it can communicate
                    // with the coup game task
                    tracing::trace!("Sending players their channels");
                    for (sender, channel) in connections.into_iter().zip(player_half) {
                        sender.send(channel).await.unwrap();
                    }
                }
            },
            Some(Disconnected { addr }) = disconnected.recv() => {
                tracing::error!(addr = %addr, "Received player disconnect on dispatcher, restarting game");
                broadcaster.send(BroadcastMessage::GameCancelled).unwrap();
            }
        }
    }
}
