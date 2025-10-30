use crate::dispatcher::PlayerHalf;
use overthrow_types::{Info, PlayerView};
use tokio::select;
use tokio::sync::mpsc::Receiver;

use super::dispatcher::GameHalf;
use futures::future::{join_all, select_all};
use overthrow_engine::action::{Action, Block, Blocks, Challenge, Reaction};
use overthrow_engine::deck::{Card, Hand};
use overthrow_engine::machine::{
    ActionKind, BlockState, ChallengeState, ChooseOneFromThree, ChooseOneFromThreeState,
    ChooseTwoFromFour, ChooseTwoFromFourState, ChooseVictimCard, ChooseVictimCardState, CoupGame,
    EndState, GameState as CoupGameState, OnlyBlockable, OnlyBlockableState, OnlyChallengeable,
    OnlyChallengeableState, Outcome, Reactable, ReactableState, Safe, SafeState, Summary, Wait,
    WaitState,
};
use overthrow_engine::player_map::PlayerMap;
use overthrow_engine::players::PlayerId;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{
    broadcast::{self, error::SendError as BroadcastError},
    mpsc::error::SendError as MpscError,
};
use tracing::{instrument, trace};

#[derive(Debug)]
pub struct Pass;

#[derive(Debug)]
pub struct PlayerGameInfo {
    pub id: PlayerId,
    pub broadcast_receiver: broadcast::Receiver<BroadcastMessage>,
    pub info: Receiver<Info>,
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
    Outcome(Outcome),
    End(Summary),
    GameCancelled,
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

    loop {
        use CoupGameState as State;
        let handles = ChannelHandles {
            player_channels: &mut player_channels,
            broadcaster: &broadcaster,
        };

        // round has started, so we can broadcast the game info to all of the players
        if let State::Wait(game) = &game_state {
            let info = game.info();

            tracing::trace!(info = ?info, "Broadcasting game info to each player");
            for (id, _) in info.players.alive() {
                let views = get_player_views_for(id, info.players);
                let (_, channels) = &handles.player_channels[&id];
                let info = Info {
                    player_views: views,
                    current_player: info.current_player,
                    coins_remaining: info.coins_remaining,
                };

                channels.info.send(info).await?;
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
    }
}

fn get_player_views_for(player_id: PlayerId, players: &PlayerMap) -> HashMap<PlayerId, PlayerView> {
    let alive_views = players.alive().map(|(id, player)| {
        let revealed_cards = match player.hand() {
            Hand::Full(..) => Vec::new(),
            Hand::Last { dead, .. } => vec![dead],
        };

        let view = if player_id == id {
            PlayerView::Me {
                name: player.name().to_owned(),
                coins: player.coins().amount(),
                hand: player.hand().clone(),
            }
        } else {
            PlayerView::Other {
                name: player.name().to_owned(),
                coins: player.coins().amount(),
                revealed_cards,
            }
        };

        (id, view)
    });

    let dead_views = players.dead().map(|(id, player)| {
        let view = PlayerView::Other {
            name: player.name().to_owned(),
            coins: 0,
            revealed_cards: player.revealed().into(),
        };
        (id, view)
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
    let challenges = game.challenges();

    // send challenges to client handlers
    trace!("Sending challenges to client handlers");
    send_challenges(challenges.all(), player_channels).await?;

    let (challenges, passes): (Vec<_>, Vec<_>) = player_channels
        .iter_mut()
        .filter_map(|(id, channels)| (*id != challenges.actor()).then_some(channels))
        .map(|(_, receivers)| {
            (
                Box::pin(receivers.challenge.recv()),
                Box::pin(receivers.pass.recv()),
            )
        })
        .collect();

    let challenges = select_all(challenges);
    let passes = join_all(passes);

    select! {
        // if someone challenges within the 10 second window
        (Some(challenge), _, _) = challenges => {
            let game = game.challenge(challenge);
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(game.advance())
        },
        // all potential challengers have passed on challenging
        _ = passes => {
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(game.advance())
        },
    }
}

async fn handle_reactable(
    game: CoupGame<Reactable>,
    ChannelHandles {
        player_channels,
        broadcaster,
    }: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    let reactions = game.reactions();

    // send client handlers all reactions
    trace!("Sending reactions to client handlers");
    send_reactions(reactions.all(), player_channels).await?;

    let blocker = reactions.block().blocker();
    let actor = reactions.actor();

    let mut block = None;
    let (challenges, passes): (Vec<_>, Vec<_>) = player_channels
        .iter_mut()
        .filter(|(id, _)| **id != actor)
        .flat_map(|(id, (_, receivers))| {
            // no non-constant split mutable borrows of hashmap :(
            if *id == blocker {
                let fut = Box::pin(receivers.block.recv());
                block = Some(fut);
            }
            Some((
                Box::pin(receivers.challenge.recv()),
                Box::pin(receivers.pass.recv()),
            ))
        })
        .collect();

    let block = block.expect("Must be set");
    let passes = join_all(passes);
    let challenges = select_all(challenges);

    // race between the victim blocking, anyone challenging, and a 10 second timeout
    select! {
        // someone blocks within 10 second timeframe
        Some(block) = block => {
            // FIXME: handle challenging a block
            let game = game.block(block);
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(CoupGameState::Wait(game.advance()))
        },
        // someone challenges within 10 second timeframe
        (Some(challenge), _, _) = challenges => {
            let game = game.challenge(challenge);
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(game.advance())
        },
        // all potential reactors pass
        _ = passes => {
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(game.advance())
        }
    }
}

async fn handle_blockable(
    game: CoupGame<OnlyBlockable>,
    ChannelHandles {
        player_channels,
        broadcaster,
    }: ChannelHandles<'_>,
) -> Result<CoupGameState> {
    let blocks = game.blocks();

    // send client handlers the possible blocks
    trace!("Sending blocks to client handlers");
    send_blocks(blocks.all(), player_channels).await?;

    let (blocks, passes): (Vec<_>, Vec<_>) = player_channels
        .iter_mut()
        .filter_map(|(id, (_, receivers))| (*id != blocks.actor()).then_some(receivers))
        .map(|receivers| {
            (
                Box::pin(receivers.block.recv()),
                Box::pin(receivers.pass.recv()),
            )
        })
        .collect();

    let blocks = select_all(blocks);
    let passes = join_all(passes);

    // if someone blocks within the 10 second window
    select! {
        (Some(block), _, _) = blocks => {
            // FIXME: handle challenging a block
            let game = game.block(block);
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;

            Ok(CoupGameState::Wait(game.advance()))
        },
        _ = passes => {
            broadcaster.send(BroadcastMessage::Outcome(game.outcome()))?;
            Ok(CoupGameState::Wait(game.advance()))
        },
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
