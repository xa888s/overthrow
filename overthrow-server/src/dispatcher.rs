use crate::Disconnected;
use crate::game::{BroadcastMessage, Pass, PlayerCommunicationError, PlayerGameInfo};

use super::game::GameMessage;

use super::game::coup_game;
use overthrow_engine::action::{Action, Block, Challenge};
use overthrow_engine::deck::Card;
use overthrow_engine::players::PlayerId;
use overthrow_types::{Info, Summary};
use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use tokio::select;
use tokio::sync::broadcast;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::instrument;
use uuid::Uuid;

pub type PlayerHalf = (ClientChannels, Receiver<GameMessage>);
pub type GameHalf = (Sender<GameMessage>, GameChannels);
pub type TaskReceiver = Receiver<(oneshot::Sender<PlayerGameInfo>, oneshot::Sender<Uuid>)>;
type Channels = (Vec<PlayerGameInfo>, HashMap<PlayerId, GameHalf>);

// Each client has 6 senders and 1 receiver:
// The receiver receives GameMessages, while the senders are for different types of choices (Action, Challenge, choosing, etc.)
#[derive(Debug)]
pub struct ClientChannels {
    pub action: Sender<Action>,
    pub challenge: Sender<Challenge>,
    pub block: Sender<Block>,
    pub victim_card: Sender<Card>,
    pub choose_one: Sender<Card>,
    pub choose_two: Sender<[Card; 2]>,
    pub pass: Sender<Pass>,
}

// The game task has 6 receivers and 1 sender per client
// The sender is for GameMessages, and the receivers are for receiving game messages depending on which type it is
#[derive(Debug)]
pub struct GameChannels {
    pub action: Receiver<Action>,
    pub challenge: Receiver<Challenge>,
    pub block: Receiver<Block>,
    pub victim_card: Receiver<Card>,
    pub choose_one: Receiver<Card>,
    pub choose_two: Receiver<[Card; 2]>,
    pub info: Sender<Info>,
    pub pass: Receiver<Pass>,
}

// information for a given game/lobby
#[derive(Debug)]
struct GameInfo {
    channel_senders: Vec<oneshot::Sender<PlayerGameInfo>>,
    broadcaster: Arc<broadcast::Sender<BroadcastMessage>>,
    handle: Option<JoinHandle<Result<Summary, PlayerCommunicationError>>>,
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
            let (info_tx, info_rx) = mpsc::channel(1);
            let (pass_tx, pass_rx) = mpsc::channel(1);

            let senders = ClientChannels {
                action: action_tx,
                challenge: challenge_tx,
                block: block_tx,
                victim_card: victim_tx,
                choose_one: choose_one_tx,
                choose_two: choose_two_tx,
                pass: pass_tx,
            };

            let receivers = GameChannels {
                action: action_rx,
                challenge: challenge_rx,
                block: block_rx,
                victim_card: victim_rx,
                choose_one: choose_one_rx,
                choose_two: choose_two_rx,
                info: info_tx,
                pass: pass_rx,
            };

            let player_half = PlayerGameInfo {
                id,
                broadcast_receiver: broadcaster.subscribe(),
                channels: (senders, player_rx),
                info: info_rx,
            };
            let game_half = (id, (game_tx, receivers));
            (player_half, game_half)
        })
        .collect()
}

// finds a lobby to assign player to
async fn assign_to_lobby(
    lobbies: &mut HashMap<Uuid, GameInfo>,
    sender: oneshot::Sender<PlayerGameInfo>,
) -> Uuid {
    // find lobbies that haven't started their game and have space
    let lobby = lobbies
        .iter_mut()
        .find(|(_, info)| info.handle.is_none() && info.channel_senders.len() < 6);

    if let Some((game_id, info)) = lobby {
        info.channel_senders.push(sender);
        *game_id
    } else {
        // generate new lobby instead
        let uuid = Uuid::now_v7();
        lobbies.insert(
            uuid,
            GameInfo {
                channel_senders: vec![sender],
                broadcaster: Arc::new(broadcast::channel(2).0),
                // game hasn't started yet
                handle: None,
            },
        );

        uuid
    }
}

#[instrument(skip(task_receiver, disconnected))]
pub async fn dispatcher(mut task_receiver: TaskReceiver, mut disconnected: Receiver<Disconnected>) {
    // mapping to each of the lobbies/games
    let mut lobbies: HashMap<Uuid, GameInfo> = HashMap::new();
    let mut finished_games: HashMap<Uuid, GameInfo> = HashMap::new();
    loop {
        select! {
            Some((info_sender, game_id_sender)) = task_receiver.recv() => {
                // assign incoming player to a lobby
                let game_id = assign_to_lobby(&mut lobbies, info_sender).await;
                game_id_sender.send(game_id).expect("Receiver should never be dropped");

                let game = lobbies.get_mut(&game_id).expect("Guaranteed to exist");

                // check if lobby player was assigned to is ready to start
                if game.channel_senders.len() >= 2 {
                    tracing::debug!(game_id = %game_id, "Sufficient players joined, starting game");
                    let connections = mem::take(&mut game.channel_senders);

                    let (player_half, game_half) = generate_channels(connections.len(), game.broadcaster.clone());

                    // start the game task to run in the background
                    tracing::trace!(game_id = %game_id, "Starting coup game task with {} players", game_half.len());
                    game.handle = Some(tokio::spawn(coup_game(game_half, game.broadcaster.clone())));

                    // send back the player task's half of the channel, so it can communicate
                    // with the coup game task
                    tracing::trace!(game_id = %game_id, "Sending players their channels");
                    for (sender, channel) in connections.into_iter().zip(player_half) {
                        sender.send(channel).unwrap();
                    }
                }
            },
            Some(Disconnected { addr, game_id }) = disconnected.recv() => {
                tracing::error!(addr = %addr, game_id = %game_id, "Received player disconnect on dispatcher, ending game");
                // clean up should only happen once
                let Some(finished_game) = lobbies.remove(&game_id) else { continue };

                if finished_game.broadcaster.send(BroadcastMessage::GameCancelled).is_err() {
                    tracing::error!(culprit = %addr, game_id = %game_id, "No players left connected");
                }

                // abort game to make sure it doesn't keep waiting to progress
                if let Some(handle) = &finished_game.handle {
                    tracing::trace!(culprit = %addr, game_id = %game_id, "Aborting game task");
                    handle.abort();
                }

                // add to finished games map
                finished_games.insert(game_id, finished_game);
            }
        }
    }
}
