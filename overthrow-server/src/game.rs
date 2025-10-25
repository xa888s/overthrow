use crate::dispatcher::PlayerHalf;
use overthrow_types::{Info, PlayerView};
use tokio::select;
use tokio::time::{Instant, timeout, timeout_at};

use super::dispatcher::GameHalf;
use futures::future::select_all;
use overthrow_engine::action::{Action, Block, Blocks, Challenge, Reaction};
use overthrow_engine::deck::{Card, DeadCard, Hand};
use overthrow_engine::machine::{
    ActionKind, BlockState, ChallengeState, ChooseOneFromThree, ChooseOneFromThreeState,
    ChooseTwoFromFour, ChooseTwoFromFourState, ChooseVictimCard, ChooseVictimCardState, CoupGame,
    EndState, GameState as CoupGameState, OnlyBlockable, OnlyBlockableState, OnlyChallengeable,
    OnlyChallengeableState, Outcome, Reactable, ReactableState, Safe, SafeState, Summary, Wait,
    WaitState,
};
use overthrow_engine::players::{PlayerId, Players};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{
    broadcast::{self, error::SendError as BroadcastError},
    mpsc::error::SendError as MpscError,
};
use tracing::instrument;

#[derive(Debug)]
pub struct PlayerGameInfo {
    pub id: PlayerId,
    pub broadcast_receiver: broadcast::Receiver<BroadcastMessage>,
    pub channels: PlayerHalf,
}

#[derive(Debug)]
pub enum Choices {
    Actions(Vec<Action>),
    Challenge(Challenge),
    Block(Blocks),
    Reactions(Vec<Reaction>),
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
pub enum GameMessage {
    ChooseAction(Choices),
    ChooseVictim([Card; 2]),
    ChooseOneFromThree([Card; 3]),
    ChooseTwoFromFour([Card; 4]),
}

#[derive(Debug, Clone)]
pub enum BroadcastMessage {
    Info(Info),
    Outcome(Outcome),
    End(Summary),
    GameCancelled,
    ReactionTimeout,
}

#[derive(Debug, Clone)]
pub struct PlayerCommunicationError;
impl<T> From<MpscError<T>> for PlayerCommunicationError {
    fn from(_: MpscError<T>) -> Self {
        PlayerCommunicationError
    }
}

impl<T> From<BroadcastError<T>> for PlayerCommunicationError {
    fn from(_: BroadcastError<T>) -> Self {
        PlayerCommunicationError
    }
}

type Result<T> = std::result::Result<T, PlayerCommunicationError>;

impl From<Choices> for GameMessage {
    fn from(choices: Choices) -> Self {
        GameMessage::ChooseAction(choices)
    }
}

#[derive(Debug)]
struct ChannelHandles<'a> {
    player_channels: &'a mut HashMap<PlayerId, GameHalf>,
    broadcaster: &'a broadcast::Sender<BroadcastMessage>,
}

// HashMap will contain senders and receivers for the corresponding PlayerId (which will in turn be attended to by a specific task)
// This function will run until the game is over, where it will send an end game message to all player tasks
#[instrument(skip(player_channels))]
pub async fn coup_game(
    mut player_channels: HashMap<PlayerId, GameHalf>,
    broadcaster: Arc<broadcast::Sender<BroadcastMessage>>,
) -> Result<Summary> {
    let mut game_state = CoupGameState::Wait(CoupGame::with_count(player_channels.len()));

    let summary = loop {
        use CoupGameState as State;
        let handles = ChannelHandles {
            player_channels: &mut player_channels,
            broadcaster: &broadcaster,
        };

        // round has started, so we can broadcast the game info to all of the players
        if let State::Wait(game) = &game_state {
            let info = game.info();
            let player_views = get_player_views(info.players);
            let info = Info {
                player_views,
                current_player: info.current_player,
                coins_remaining: info.coins_remaining,
            };

            tracing::trace!(info = ?info, "Broadcasting game info to all players");
            if broadcaster.send(BroadcastMessage::Info(info)).is_err() {
                tracing::error!("Failed to broadcase info to players (probably all disconnected)");
            }
        }

        let next_game_state = match game_state {
            State::Wait(coup_game) => handle_wait(coup_game, handles).await,
            State::ChooseVictimCard(coup_game) => choose_victim_card(coup_game, handles).await,
            State::ChooseOneFromThree(coup_game) => choose_one(coup_game, handles).await,
            State::ChooseTwoFromFour(coup_game) => choose_two(coup_game, handles).await,
            State::End(coup_game) => {
                let summary = coup_game.summary();
                tracing::debug!(winner = ?summary.winner, "Game finished successfully");
                // end game for all players
                broadcaster.send(BroadcastMessage::End(summary)).unwrap();
                if broadcaster.send(BroadcastMessage::End(summary)).is_err() {
                    tracing::error!(
                        "Failed to broadcast info to players (probably all disconnected)"
                    );
                }
                break Ok(summary);
            }
        };

        match next_game_state {
            Ok(next_game_state) => game_state = next_game_state,
            Err(e) => {
                tracing::error!("Failed to communicate with client (probably disconnected)");
                break Err(e);
            }
        }
    };

    summary
}

fn get_player_views(players: &Players) -> HashMap<PlayerId, PlayerView> {
    let alive_views = players.alive().iter().map(|(id, player)| {
        let revealed_cards = match player.hand() {
            Hand::Full(..) => Vec::new(),
            Hand::Last(_, dead) => vec![dead.card()],
        };

        (
            *id,
            PlayerView {
                name: player.name().to_owned(),
                coins: player.coins().amount(),
                revealed_cards,
            },
        )
    });

    let dead_views = players.dead().iter().map(|(id, player)| {
        let revealed_cards = player.cards().iter().map(DeadCard::card).collect();
        (
            *id,
            PlayerView {
                name: player.name().to_owned(),
                coins: 0,
                revealed_cards,
            },
        )
    });

    alive_views.chain(dead_views).collect()
}

#[instrument(skip_all)]
async fn choose_victim_card(
    game: CoupGame<ChooseVictimCard>,
    handles: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    let choices = game.choices();
    let victim = game.victim();
    tracing::debug!(victim = ?victim, choices = ?choices, "Choosing victim card");

    let (sender, receivers) = handles
        .player_channels
        .get_mut(&victim)
        .expect("Must exist");

    sender.send(GameMessage::ChooseVictim(choices)).await?;

    let choice = receivers
        .victim_card
        .recv()
        .await
        .ok_or(PlayerCommunicationError)?;
    tracing::debug!(victim = ?victim, choice = ?choice, possible_choices = ?choices, "Received choice");
    Ok(CoupGameState::Wait(game.advance(choice)))
}

#[instrument(skip_all)]
async fn choose_one(
    game: CoupGame<ChooseOneFromThree>,
    handles: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    let choices = game.choices();
    let actor = game.actor();
    tracing::debug!(actor = ?actor, choices = ?choices, "Exchanging one card from three");

    let (sender, receivers) = handles.player_channels.get_mut(&actor).expect("Must exist");

    sender
        .send(GameMessage::ChooseOneFromThree(choices))
        .await?;

    let choice = receivers
        .choose_one
        .recv()
        .await
        .ok_or(PlayerCommunicationError)?;
    tracing::debug!(actor = ?actor, choice = ?choice, possible_choices = ?choices, "Received choice");
    Ok(CoupGameState::Wait(game.advance(choice)))
}

#[instrument(skip_all)]
async fn choose_two(
    game: CoupGame<ChooseTwoFromFour>,
    handles: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    let choices = game.choices();
    let actor = game.actor();
    tracing::debug!(actor = ?actor, choices = ?choices, "Exchanging two cards from four");

    let (sender, receivers) = handles.player_channels.get_mut(&actor).expect("Must exist");

    sender.send(GameMessage::ChooseTwoFromFour(choices)).await?;

    let chosen = receivers
        .choose_two
        .recv()
        .await
        .ok_or(PlayerCommunicationError)?;
    tracing::debug!(actor = ?actor, choice = ?chosen, possible_choices = ?choices, "Received choice");
    Ok(CoupGameState::Wait(game.advance(chosen)))
}

async fn handle_wait(game: CoupGame<Wait>, handles: ChannelHandles<'_>) -> Result<CoupGameState> {
    let actions: Vec<Action> = game.actions().all().cloned().collect();
    let current_player = game.info().current_player;

    let (sender, receivers) = handles
        .player_channels
        .get_mut(&current_player)
        .expect("Must exist");

    tracing::trace!(actions = ?actions, "Sending choices to client");
    sender.send(Choices::Actions(actions).into()).await?;

    let choice = receivers
        .action
        .recv()
        .await
        .ok_or(PlayerCommunicationError)?;
    tracing::trace!(chosen_action = ?choice, "Received choice");

    use ActionKind as A;
    match game.play(choice) {
        A::Safe(coup_game) => handle_safe(coup_game, handles.broadcaster).await,
        A::OnlyChallengeable(coup_game) => handle_challengeable(coup_game, handles).await,
        A::OnlyBlockable(coup_game) => handle_blockable(coup_game, handles).await,
        A::Reactable(coup_game) => handle_reactable(coup_game, handles).await,
    }
}

async fn handle_safe(
    game: CoupGame<Safe>,
    broadcaster: &broadcast::Sender<BroadcastMessage>,
) -> Result<CoupGameState> {
    // safe actions always succeed, so we broadcast outcome and continue game
    let outcome = game.outcome();
    broadcaster.send(BroadcastMessage::Outcome(outcome))?;

    Ok(game.advance())
}

async fn handle_challengeable(
    game: CoupGame<OnlyChallengeable>,
    ChannelHandles {
        player_channels,
        broadcaster,
    }: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    send_challenges(game.challenges().all(), player_channels).await?;

    let challenges = player_channels
        .values_mut()
        .map(|(_, receivers)| Box::pin(receivers.challenge.recv()));

    // if someone challenges within the 10 second window
    if let Ok((Some(challenge), _, _)) =
        timeout(Duration::from_secs(10), select_all(challenges)).await
    {
        let game = game.challenge(challenge);
        broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;

        return Ok(game.advance());
    }

    // since no one has challenged, the action must continue
    broadcaster.send(BroadcastMessage::ReactionTimeout)?;
    Ok(game.advance())
}

async fn handle_reactable(
    game: CoupGame<Reactable>,
    ChannelHandles {
        player_channels,
        broadcaster,
    }: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    send_reactions(game.reactions().all(), player_channels).await?;

    let blocks = game.reactions().block().clone();
    let blocker_id = blocks.blocker_id();

    // normally wouldn't need to allocate a vector, but we can't split player_channel
    // borrows
    let (Some(blocker), challenges) = player_channels.iter_mut().fold(
        (None, Vec::new()),
        |(mut blocker, mut challenges), (id, (_, receivers))| {
            if *id == blocker_id {
                blocker = Some(receivers.block.recv());
            }
            challenges.push(Box::pin(receivers.challenge.recv()));

            (blocker, challenges)
        },
    ) else {
        unreachable!()
    };

    // set 10 second timer
    let delay = Instant::now() + Duration::from_secs(10);

    // race between the victim blocking, anyone challenging, and a 10 second timeout
    select! {
        Ok(Some(block)) = timeout_at(delay, blocker) => {
            let game = game.block(block);
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(CoupGameState::Wait(game.advance()))
        },
        Ok((Some(challenge), _, _)) = timeout_at(delay, select_all(challenges)) => {
            let game = game.challenge(challenge);
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(game.advance())
        },
        else => {
            broadcaster.send(BroadcastMessage::ReactionTimeout)?;
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(game.advance())
        },
    }
}

async fn handle_blockable(
    game: CoupGame<OnlyBlockable>,
    ChannelHandles {
        player_channels,
        broadcaster,
    }: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    send_blocks(game.blocks().all(), player_channels).await?;

    let blocks = player_channels
        .values_mut()
        .map(|(_, receivers)| Box::pin(receivers.block.recv()));

    // if someone blocks within the 10 second window
    if let Ok((Some(block), _, _)) = timeout(Duration::from_secs(10), select_all(blocks)).await {
        let game = game.block(block);
        broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;

        Ok(CoupGameState::Wait(game.advance()))
    } else {
        // since no one has blocked, the action must continue
        broadcaster.send(BroadcastMessage::ReactionTimeout)?;
        Ok(CoupGameState::Wait(game.advance()))
    }
}

async fn send_blocks(
    blocks: &HashMap<PlayerId, Block>,
    player_channels: &mut HashMap<PlayerId, GameHalf>,
) -> Result<()> {
    for (id, block) in blocks {
        let (sender, _) = player_channels.get_mut(id).expect("Must exist");
        sender
            .send(Choices::Block(Blocks::Other(block.clone())).into())
            .await?;
    }

    Ok(())
}

async fn send_challenges(
    challenges: &HashMap<PlayerId, Challenge>,
    player_channels: &mut HashMap<PlayerId, GameHalf>,
) -> Result<()> {
    for (id, challenge) in challenges {
        let (sender, _) = player_channels.get_mut(id).expect("Must exist");
        sender
            .send(Choices::Challenge(challenge.clone()).into())
            .await?;
    }

    Ok(())
}

async fn send_reactions(
    reactions: HashMap<PlayerId, Vec<Reaction>>,
    player_channels: &mut HashMap<PlayerId, GameHalf>,
) -> Result<()> {
    for (id, reaction) in reactions {
        let (sender, _) = player_channels.get_mut(&id).expect("Must exist");
        sender.send(Choices::Reactions(reaction).into()).await?;
    }

    Ok(())
}
