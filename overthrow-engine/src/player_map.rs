use std::ops::Index;

use crate::{
    action::{
        Act, Action, Block, BlockableAct, Blocks, Challenge, ChallengeableAct, PossibleActions,
        PossibleBlocks, PossibleChallenges, PossibleReactions, ReactableAct,
    },
    coins::PlayerCoins,
    current_player::CurrentPlayer,
    deck::{BlockStealClaim, Card, Hand},
    players::PlayerId,
};
use arrayvec::ArrayVec;

pub const MAX_PLAYER_COUNT: usize = 6;

#[derive(Debug, Clone)]
pub struct AlivePlayerData {
    pub(crate) name: String,
    pub(crate) coins: PlayerCoins,
    pub(crate) hand: Hand,
}

impl AlivePlayerData {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn hand(&self) -> Hand {
        self.hand.clone()
    }

    pub fn coins(&self) -> PlayerCoins {
        self.coins.clone()
    }

    pub fn can_coup(&self) -> bool {
        self.coins.amount() >= 7
    }

    pub fn can_assasinate(&self) -> bool {
        self.coins.amount() >= 3
    }

    pub fn can_be_stolen_from(&self) -> bool {
        self.coins.amount() >= 2
    }
}

#[derive(Debug, Clone)]
pub struct DeadPlayerData {
    pub(crate) name: String,
    pub(crate) revealed: [Card; 2],
}

impl DeadPlayerData {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn revealed(&self) -> [Card; 2] {
        self.revealed
    }
}

#[derive(Debug, Clone)]
pub enum Player {
    Alive(AlivePlayerData),
    Dead(DeadPlayerData),
}

impl Player {
    fn alive(name: String, coins: PlayerCoins, hand: Hand) -> Player {
        let data = AlivePlayerData { name, coins, hand };
        Player::Alive(data)
    }

    fn dead(name: String, revealed: [Card; 2]) -> Player {
        let data = DeadPlayerData { name, revealed };
        Player::Dead(data)
    }
}

#[derive(Debug)]
pub struct PlayerMap {
    players: ArrayVec<Player, MAX_PLAYER_COUNT>,
    current: CurrentPlayer,
}

impl PlayerMap {
    pub fn new(players: impl IntoIterator<Item = (String, PlayerCoins, Hand)>) -> PlayerMap {
        let players = players
            .into_iter()
            .map(|(name, coins, hand)| Player::alive(name, coins, hand));

        // panics if bigger than 6
        let players = ArrayVec::<Player, MAX_PLAYER_COUNT>::from_iter(players);

        // and panics if less than 2
        let count = players.len();
        assert!(count >= 2);
        PlayerMap {
            players,
            current: CurrentPlayer::new(count),
        }
    }

    // total player count (dead and alive)
    pub fn count(&self) -> usize {
        self.players.len()
    }

    // iterator over all alive players (unpacked)
    pub fn alive(&self) -> impl Iterator<Item = (PlayerId, &AlivePlayerData)> {
        self.all().filter_map(|(id, p)| match p {
            Player::Alive(data) => Some((id, data)),
            Player::Dead(..) => None,
        })
    }

    // iterator over all dead players (unpacked)
    pub fn dead(&self) -> impl Iterator<Item = (PlayerId, &DeadPlayerData)> {
        self.all().filter_map(|(id, p)| match p {
            Player::Dead(data) => Some((id, data)),
            Player::Alive(..) => None,
        })
    }

    pub fn all(&self) -> impl Iterator<Item = (PlayerId, &Player)> {
        PlayerId::iter().zip(&self.players)
    }

    // returns coins to be replaced in pile if successful, otherwise, returns error
    pub(crate) fn kill(&mut self, id: PlayerId) -> PlayerCoins {
        let index = id as usize - 1;
        let Player::Alive(data) = self.players[index].clone() else {
            unreachable!("Player should exist")
        };

        let Hand::Last { alive, dead } = data.hand else {
            unreachable!("Player should only be killed when on their last card")
        };

        let player = Player::dead(data.name, [alive, dead]);

        self.players[index] = player;
        self.current.kill(id);

        data.coins
    }

    pub(crate) fn end_turn(&mut self) {
        self.current.end_turn();
    }

    pub fn current_player(&self) -> PlayerId {
        self.current.current()
    }

    // returns last player's player id if there is only one player left (equivalent to saying game is over)
    pub(crate) fn game_over(&self) -> Option<PlayerId> {
        let mut alive_players = PlayerId::iter()
            .zip(&self.players)
            .filter(|(_, p)| matches!(p, Player::Alive { .. }));

        let (last, _) = alive_players
            .next()
            .expect("There should always be at least one player");

        alive_players.next().is_none().then_some(last)
    }

    pub(crate) fn exchange(&mut self, id: PlayerId, hand: Hand) {
        self.as_alive_mut(id).hand = hand;
    }

    pub(crate) fn hand_for(&self, id: PlayerId) -> Hand {
        self.as_alive(id).hand.clone()
    }

    pub(crate) fn get_coins_for(&self, id: PlayerId) -> PlayerCoins {
        self.as_alive(id).coins.clone()
    }

    pub(crate) fn set_coins_for(&mut self, id: PlayerId, coins: PlayerCoins) {
        self.as_alive_mut(id).coins = coins;
    }

    pub(crate) fn has_card(&self, id: PlayerId, card: Card) -> bool {
        self.as_alive(id).hand.has_card(card)
    }

    // treat player as if they were alive, panicking otherwise
    fn as_alive(&self, id: PlayerId) -> &AlivePlayerData {
        let Player::Alive(data) = &self[id] else {
            unreachable!("Player should be alive")
        };

        data
    }

    // treat player as if they were alive, panicking otherwise
    fn as_alive_mut(&mut self, id: PlayerId) -> &mut AlivePlayerData {
        let index = id as usize - 1;
        let Player::Alive(data) = &mut self.players[index] else {
            unreachable!("Player should be alive")
        };

        data
    }

    // rules on a challenge
    pub(crate) fn challenge_winner(
        &self,
        actor: PlayerId,
        challenger: PlayerId,
        claim: Card,
    ) -> PlayerId {
        if self.has_card(actor, claim) {
            actor
        } else {
            challenger
        }
    }

    fn map_all_but<A, Map>(
        &self,
        actor: PlayerId,
        mut map: Map,
    ) -> impl Iterator<Item = (PlayerId, A)> + use<'_, Map, A>
    where
        Map: FnMut(PlayerId) -> A,
    {
        self.all()
            .filter(move |(id, p)| *id != actor && matches!(p, Player::Alive(..)))
            .map(move |(id, _)| (id, map(id)))
    }

    // different types of steal blocks (as ambassador or captain)
    fn block_steals(actor: PlayerId, blocker: PlayerId) -> Blocks {
        let kind = BlockableAct::Steal {
            victim: blocker,
            claim: BlockStealClaim::Ambassador,
        };
        let ambassador = Block {
            actor,
            blocker,
            kind,
        };

        let kind = BlockableAct::Steal {
            victim: blocker,
            claim: BlockStealClaim::Captain,
        };
        let captain = Block {
            actor,
            blocker,
            kind,
        };

        Blocks::Steal(ambassador, captain)
    }

    // generates challenges against actor's challengeable act
    pub(crate) fn generate_challenges_against(
        &self,
        actor: PlayerId,
        action: ChallengeableAct,
    ) -> PossibleChallenges {
        let challenge_from_id = |challenger| Challenge {
            actor,
            challenger,
            kind: action.clone(),
        };

        let challenges = self.map_all_but(actor, challenge_from_id).collect();

        PossibleChallenges { challenges, actor }
    }

    // generates blocks for the only kind of block that anyone can do (foreign aid)
    pub(crate) fn generate_blocks_against(&self, actor: PlayerId) -> PossibleBlocks {
        let block_from_id = |blocker| Block {
            actor,
            blocker,
            kind: BlockableAct::ForeignAid,
        };

        let blocks = self.map_all_but(actor, block_from_id).collect();

        PossibleBlocks { blocks, actor }
    }

    // generates reactions for against actor's action
    #[allow(clippy::toplevel_ref_arg)]
    pub(crate) fn generate_reactions_against(
        &self,
        actor: PlayerId,
        ref action: ReactableAct,
    ) -> PossibleReactions {
        let challenge_from_id = |challenger| Challenge {
            actor,
            challenger,
            kind: action.into(),
        };

        let challenge = self.map_all_but(actor, challenge_from_id).collect();

        let block = match *action {
            ReactableAct::Steal { victim } => PlayerMap::block_steals(actor, victim),
            ReactableAct::Assassinate { victim } => Blocks::Other(Block {
                actor,
                blocker: victim,
                kind: BlockableAct::Assassinate { victim },
            }),
        };

        PossibleReactions {
            block,
            challenge,
            actor,
        }
    }

    // generates the possible actions for id
    pub(crate) fn generate_actions_for(&self, id: PlayerId) -> PossibleActions {
        const BASIC_ACTS: [Act; 4] = [Act::ForeignAid, Act::Income, Act::Tax, Act::Exchange];
        let action_from_act = move |act| Action::new(id, act);

        let assassinations = self
            .potential_assasination_victims(id)
            .map(|victim| action_from_act(Act::Assassinate { victim }))
            .collect();

        let coups = self
            .potential_coup_victims(id)
            .map(|victim| action_from_act(Act::Coup { victim }))
            .collect();

        let steal = self
            .potential_steal_victims(id)
            .map(|victim| action_from_act(Act::Steal { victim }))
            .collect();

        let basic = BASIC_ACTS.map(action_from_act).into_iter().collect();

        PossibleActions {
            actor: id,
            assassinations,
            coups,
            steal,
            basic,
        }
    }

    // returns an iterator of the ids of possible steal victims
    fn potential_steal_victims(&self, actor: PlayerId) -> impl Iterator<Item = PlayerId> + use<'_> {
        self.alive()
            .filter(move |(id, player)| *id != actor && player.can_be_stolen_from())
            .map(|(id, _)| id)
    }

    // returns an iterator of the ids of possible coup victims
    fn potential_coup_victims(&self, actor: PlayerId) -> impl Iterator<Item = PlayerId> + use<'_> {
        let can_coup = self.as_alive(actor).can_coup();
        let possible_victims = if can_coup { self.alive().count() } else { 0 };

        self.alive()
            .filter(move |(id, _)| *id != actor)
            .map(|(id, _)| id)
            .take(possible_victims)
    }

    // returns an iterator of the ids of possible assasination victims
    fn potential_assasination_victims(
        &self,
        actor: PlayerId,
    ) -> impl Iterator<Item = PlayerId> + use<'_> {
        let can_assasinate = self.as_alive(actor).can_assasinate();
        let possible_victims = if can_assasinate {
            self.alive().count()
        } else {
            0
        };

        self.alive()
            .filter(move |(id, _)| *id != actor)
            .map(|(id, _)| id)
            .take(possible_victims)
    }
}

impl Index<PlayerId> for PlayerMap {
    type Output = Player;

    fn index(&self, index: PlayerId) -> &Player {
        // player ids start at 1
        let index = index as usize - 1;
        &self.players[index]
    }
}
