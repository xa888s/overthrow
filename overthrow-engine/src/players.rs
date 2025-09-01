use crate::action::Blocks;

use super::action::{Act, Action};
use super::action::{
    Block, BlockableAct, Challenge, ChallengeableAct, PossibleActions, PossibleBlocks,
    PossibleChallenges, PossibleReactions, ReactableAct,
};
use super::deck::{BlockStealClaim, DeadCard};

use super::coins::PlayerCoins;
use super::current_player::CurrentPlayer;
use super::deck::{Card, DeadHand, Hand};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// there can only be 6 players
#[derive(
    Debug, Hash, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, JsonSchema,
)]
pub enum PlayerId {
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
    Six = 6,
}

impl PlayerId {
    fn copy(&self) -> PlayerId {
        *self
    }

    pub fn iter() -> impl Iterator<Item = PlayerId> {
        use PlayerId::*;
        [One, Two, Three, Four, Five, Six].into_iter()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Player {
    pub(crate) name: String,
    pub(crate) coins: PlayerCoins,
    pub(crate) hand: Hand,
}

impl Player {
    pub(crate) fn new(name: String, coins: PlayerCoins, hand: Hand) -> Player {
        Player { name, coins, hand }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn coins(&self) -> &PlayerCoins {
        &self.coins
    }

    pub fn hand(&self) -> &Hand {
        &self.hand
    }

    pub(crate) fn can_coup(&self) -> bool {
        self.coins.amount() >= 7
    }

    pub(crate) fn can_assasinate(&self) -> bool {
        self.coins.amount() >= 3
    }

    pub(crate) fn can_steal_from(&self) -> bool {
        self.coins.amount() >= 2
    }

    pub(crate) fn has_card(&self, card: Card) -> bool {
        self.hand.has_card(card)
    }
}

#[derive(Debug)]
pub struct DeadPlayer {
    name: String,
    hand: DeadHand,
}

impl DeadPlayer {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn cards(&self) -> [DeadCard; 2] {
        let DeadHand(c1, c2) = &self.hand;
        [*c1, *c2]
    }
}

#[derive(Debug)]
pub enum PlayerKillError {
    NonExistentPlayer { id: PlayerId },
    AlreadyDead { id: PlayerId },
}

#[derive(Debug)]
pub enum PlayersInitError {
    TooManyPlayers { player_count: usize },
    TooFewPlayers { player_count: usize },
}

// newtype that ensures only the correct number of players are used
#[derive(Debug)]
pub struct RawPlayers(pub(crate) Vec<String>, pub(crate) u8);

impl RawPlayers {
    pub fn with_names<'name>(
        names: impl IntoIterator<Item = &'name str>,
    ) -> Result<RawPlayers, PlayersInitError> {
        let names: Vec<String> = names.into_iter().map(Into::into).collect();

        let player_count = names.len();

        if player_count > 6 {
            Err(PlayersInitError::TooFewPlayers { player_count })
        } else if player_count < 2 {
            Err(PlayersInitError::TooManyPlayers { player_count })
        } else {
            // we know: 2 <= player_count <= 6
            Ok(RawPlayers(names, player_count as u8))
        }
    }
}

#[derive(Debug)]
pub struct Players {
    alive: HashMap<PlayerId, Player>,
    dead: HashMap<PlayerId, DeadPlayer>,
    current_player: CurrentPlayer,
}

impl Players {
    pub(crate) fn with_players(players: impl IntoIterator<Item = (PlayerId, Player)>) -> Players {
        let alive = players.into_iter().collect();
        Players {
            current_player: CurrentPlayer::with_players(&alive),
            alive,
            dead: HashMap::new(),
        }
    }

    pub(crate) fn has_card(&self, actor: PlayerId, card: Card) -> bool {
        let player = self.alive.get(&actor).expect("Player should be alive");
        player.has_card(card)
    }

    pub(crate) fn challenge_winner(
        &self,
        actor: PlayerId,
        challenger: PlayerId,
        claim: Card,
    ) -> PlayerId {
        if self.has_card(actor, claim) {
            challenger
        } else {
            actor
        }
    }

    pub(crate) fn get_coins_for(&self, actor: PlayerId) -> PlayerCoins {
        let player = self.alive.get(&actor).expect("Player should be alive");
        player.coins.clone()
    }

    pub(crate) fn set_coins_for(&mut self, actor: PlayerId, coins: PlayerCoins) {
        let player = self.alive.get_mut(&actor).expect("Player should be alive");
        player.coins = coins;
    }

    pub(crate) fn hand_for(&self, actor: PlayerId) -> Hand {
        let player = self.alive.get(&actor).expect("Player should be alive");
        player.hand.clone()
    }

    pub(crate) fn exchange(&mut self, actor: PlayerId, hand: Hand) {
        let player = self.alive.get_mut(&actor).expect("Player should be alive");
        player.hand = hand;
    }

    // returns an iterator over id and value A pairs, where the id is not equal to
    // actor
    fn all_but<A, Map>(
        &self,
        actor: PlayerId,
        mut map: Map,
    ) -> impl Iterator<Item = (PlayerId, A)> + use<'_, Map, A>
    where
        Map: FnMut(PlayerId) -> A,
    {
        self.alive
            .keys()
            .filter_map(move |&id| (id != actor).then_some((id, map(id))))
    }

    pub(crate) fn generate_challenges_against(
        &self,
        actor: PlayerId,
        action: ChallengeableAct,
    ) -> PossibleChallenges {
        let challenge_from_id = move |id| Challenge {
            actor: actor.copy(),
            challenger: id,
            kind: action.clone(),
        };

        PossibleChallenges {
            challenges: self.all_but(actor, challenge_from_id).collect(),
        }
    }

    pub(crate) fn generate_blocks_against(&self, actor: PlayerId) -> PossibleBlocks {
        let block_from_id = move |id| Block {
            actor,
            blocker: id,
            kind: BlockableAct::ForeignAid,
        };
        let blocks = self.all_but(actor, block_from_id).collect();

        PossibleBlocks { blocks }
    }

    pub(crate) fn generate_reactions_against(
        &self,
        actor: PlayerId,
        action: &ReactableAct,
    ) -> PossibleReactions {
        let challenge_from_id = |id| Challenge {
            actor: actor.copy(),
            challenger: id,
            kind: action.into(),
        };
        let challenges = self.all_but(actor, challenge_from_id);

        let blocks = match action {
            ReactableAct::Assassinate { victim } => Blocks::Other(Block {
                actor,
                blocker: *victim,
                kind: BlockableAct::Assassinate { victim: *victim },
            }),
            ReactableAct::Steal { victim } => Blocks::Steal(
                Block {
                    actor,
                    blocker: *victim,
                    kind: BlockableAct::Steal {
                        victim: *victim,
                        claim: BlockStealClaim::Ambassador,
                    },
                },
                Block {
                    actor,
                    blocker: *victim,
                    kind: BlockableAct::Steal {
                        victim: *victim,
                        claim: BlockStealClaim::Captain,
                    },
                },
            ),
        };

        PossibleReactions {
            block: blocks,
            challenge: challenges.collect(),
        }
    }

    pub(crate) fn generate_actions_for(&self, player_id: PlayerId) -> PossibleActions {
        const BASIC_ACTS: [Act; 4] = [Act::ForeignAid, Act::Income, Act::Tax, Act::Exchange];
        let action_from_act = move |act| Action::new(player_id, act);

        let assassination_vics = self
            .potential_assasination_victims(player_id)
            .map(|victim_id| action_from_act(Act::Assassinate { victim: victim_id }));

        let coup_vics = self
            .potential_coup_victims(player_id)
            .map(|victim_id| action_from_act(Act::Coup { victim: victim_id }));

        let steal_vics = self
            .potential_steal_victims(player_id)
            .map(|victim_id| action_from_act(Act::Steal { victim: victim_id }));

        let basics = BASIC_ACTS.map(action_from_act);

        PossibleActions {
            current_player: player_id,
            assassinations: assassination_vics.collect(),
            coups: coup_vics.collect(),
            steal: steal_vics.collect(),
            basic: basics.into_iter().collect(),
        }
    }

    fn potential_steal_victims(&self, actor: PlayerId) -> impl Iterator<Item = PlayerId> + use<'_> {
        self.alive
            .iter()
            .filter_map(move |(&id, player)| (player.can_steal_from() && id != actor).then_some(id))
    }

    fn potential_coup_victims(&self, actor: PlayerId) -> impl Iterator<Item = PlayerId> + use<'_> {
        let victims = self.alive.keys().copied().filter(move |&id| id != actor);

        // this is because returning impl Iterator requires returning the same type in any path
        // so we crate an empty iterator (take(0)) if the actor cannot coup
        victims.take(if self.alive[&actor].can_coup() {
            self.alive.len()
        } else {
            0
        })
    }

    fn potential_assasination_victims(
        &self,
        actor: PlayerId,
    ) -> impl Iterator<Item = PlayerId> + use<'_> {
        let victims = self.alive.keys().copied().filter(move |&id| id != actor);

        // this is because returning impl Iterator requires returning the same type in any path
        // so we crate an empty iterator (take(0)) if the actor cannot coup
        victims.take(if self.alive[&actor].can_assasinate() {
            self.alive.len()
        } else {
            0
        })
    }

    pub(crate) fn is_game_over(&self) -> bool {
        // last player left wins
        self.alive.len() == 1
    }

    pub fn alive(&self) -> &HashMap<PlayerId, Player> {
        &self.alive
    }

    pub fn dead(&self) -> &HashMap<PlayerId, DeadPlayer> {
        &self.dead
    }

    pub fn current_player(&self) -> PlayerId {
        self.current_player.current()
    }

    pub fn end_turn(&mut self) {
        self.current_player.end_turn();
    }

    /// Call with .take(self.alive.len()) to make the iterator do one loop of the players
    pub fn order(&self) -> impl Iterator<Item = PlayerId> + use<'_> {
        self.current_player.order()
    }

    // returns coins to be replaced in pile if successful, otherwise, returns error
    pub(crate) fn kill(&mut self, id: PlayerId) -> Result<PlayerCoins, PlayerKillError> {
        let Player { coins, name, hand } = self.alive.remove(&id).ok_or_else(|| {
            if self.dead.contains_key(&id) {
                PlayerKillError::AlreadyDead { id }
            } else {
                PlayerKillError::NonExistentPlayer { id }
            }
        })?;

        let Hand::Last(c1, c2) = hand else {
            panic!("To die one must only have one card left")
        };

        let None = self.dead.insert(
            id,
            DeadPlayer {
                name,
                hand: DeadHand(DeadCard(c1), c2),
            },
        ) else {
            panic!("Cannot kill a player twice")
        };

        self.current_player.kill(id);

        Ok(coins)
    }
}

#[cfg(test)]
mod tests {
    use crate::machine::{CoupGame, Wait, WaitState};
    use pretty_assertions::assert_eq;
    use std::sync::LazyLock;

    use super::*;
    static BASIC_GAME: LazyLock<CoupGame<Wait>> = LazyLock::new(|| {
        let players = RawPlayers::with_names(["Dave", "Garry"]).expect("Valid length");
        CoupGame::with_players(players)
    });

    #[test]
    fn basic_generate_actions() {
        // start basic game with two players
        let game = &*BASIC_GAME;
        let actions = game.actions();

        let player_one_actions = PossibleActions {
            current_player: PlayerId::One,
            assassinations: Vec::new(),
            coups: Vec::new(),
            steal: [Action::new(
                PlayerId::One,
                Act::Steal {
                    victim: PlayerId::Two,
                },
            )]
            .into(),
            basic: [
                Action::new(PlayerId::One, Act::ForeignAid),
                Action::new(PlayerId::One, Act::Income),
                Action::new(PlayerId::One, Act::Tax),
                Action::new(PlayerId::One, Act::Exchange),
            ]
            .into(),
        };

        assert_eq!(actions, &player_one_actions);
    }
}
