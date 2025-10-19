use crate::action::BlockableAct;
use crate::action::ChallengeableAct;

use super::action;
use super::action::Action;
use super::action::PossibleActions;
use super::action::PossibleReactions;
use super::coins::CoinPile;
use super::coins::Deposit;
use super::coins::Withdrawal;
use super::deck::Hand;
use super::deck::{Card, Deck};
use super::players::{PlayerId, Players, RawPlayers};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use typestate::typestate;

#[derive(Debug)]
pub struct GameInfo<'state> {
    pub players: &'state Players,
    pub current_player: PlayerId,
    pub coins_remaining: u8,
    pub deck: &'state [Card],
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
pub struct Summary {
    pub winner: PlayerId,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
pub enum Outcome {
    GainCoins { actor: PlayerId, amount: u8 },
    LoseCoins { actor: PlayerId, amount: u8 },
    LosesInfluence { victim: PlayerId },
    ExchangesCards { actor: PlayerId },
    LoseTurn { victim: PlayerId },
}

#[derive(Debug)]
pub(crate) struct CoupData {
    pub(crate) players: Players,
    pub(crate) coins: CoinPile,
    pub(crate) deck: Deck,
}

// Typestate that describes the entire Coup state loop
// You can view the following as a decision tree that shows possible paths the state machine can take between states:
//
// Wait -> Basic             -> Challenge -> Wait/End
//                           -> Block -> Challenge -> Wait/End
//                                    -> Wait
//
//      -> Reactable         -> Challenge -> Wait/End
//                           -> Block -> Challenge -> Wait/End
//                                    -> ChooseVictimCard -> Wait/End
//                                    -> Wait/End
//                           -> ChooseVictimCard -> Wait/End
//                           -> Wait/End
//
//      -> OnlyChallengeable -> Challenge -> Wait/End
//                           -> Wait/End
//
//      -> OnlyBlockable     -> Block -> Wait
//                           -> Wait
//
//      -> Safe              -> Wait/End
//
//      -> ExchangeOne       -> Challenge -> Wait/End
//                           -> Block -> Challenge -> Wait/End
//                                    -> ChooseOneFromThree -> Wait
//                                    -> Wait
//                           -> ChooseOneFromThree -> Wait
//                           -> Wait
//
//      -> ExchangeTwo       -> Challenge -> Wait/End
//                           -> Block -> Challenge -> Wait/End
//                                    -> ChooseTwoFromFour -> Wait
//                                    -> Wait
//                           -> ChooseTwoFromFour -> Wait
//                           -> Wait
//
// End (goes nowhere)
//
// Note that not all state paths can lead to the End state, only states that assasinate/coup, or challenges, can cause the game to end
// Each state has corresponding methods only available to that state:
//
// Base:
//     Wait => base state, current player must choose an action
// Actions:
//     Basic => can be blocked or challenged, these are for actions that do not require player input to proceed
//     Assassinate => can be blocked or challenged, this can require player interaction when deciding which card to give up (if assassination succeeds and target has two cards)
//     Coup => coups cannot be countered, so they are a dead end in terms of player choice
//     Exchange(One/Two) => can be blocked or challenged
// Reactions:
//     Challenge => another player has challenged an action
//     Block => another player has blocked an action
// End:
//     End => once only one player remains, this ends the game and returns a summary of the game
#[typestate]
#[rustfmt::skip]
pub(crate) mod game {
    use super::action::{BlockableAct, ChallengeableAct, OnlyChallengeableAct, PossibleBlocks, PossibleChallenges, ReactableAct, SafeAct};
    use super::*;

    #[automaton]
    pub struct CoupGame {
        // we box this data so that moves are cheap
        pub(crate) data: Box<CoupData>
    }

    #[state] pub struct Wait {
        pub(crate) possible_actions: PossibleActions,
    }

    #[state] pub struct Safe { 
        pub(crate) actor: PlayerId,
        pub(crate) kind: SafeAct,
    }
    #[state] pub struct OnlyChallengeable {
        pub(crate) possible_challenges: PossibleChallenges,
        pub(crate) actor: PlayerId,
        pub(crate) kind: OnlyChallengeableAct,
    }
    #[state] pub struct OnlyBlockable {
        pub(crate) possible_blocks: PossibleBlocks,
        pub(crate) actor: PlayerId,
    }
    #[state] pub struct Reactable {
        pub(crate) possible_reactions: PossibleReactions,
        pub(crate) actor: PlayerId,
        pub(crate) kind: ReactableAct,
    }
    #[state] pub struct ChooseVictimCard {
        pub(crate) victim: PlayerId,
        pub(crate) choices: [Card; 2],
    }
    #[state] pub struct ChooseOneFromThree {
        pub(crate) actor: PlayerId,
        pub(crate) choices: [Card; 3],
    }
    #[state] pub struct ChooseTwoFromFour {
        pub(crate) actor: PlayerId,
        pub(crate) choices: [Card; 4],
    }
    #[state] pub struct Challenge {
        pub(crate) actor: PlayerId,
        pub(crate) challenger: PlayerId,
        pub(crate) kind: ChallengeableAct,
    }
    #[allow(dead_code)]
    #[state] pub struct Block {
        pub(crate) possible_challenges: PossibleChallenges,
        pub(crate) actor: PlayerId,
        pub(crate) blocker: PlayerId,
        pub(crate) kind: BlockableAct,
    }
    #[state] pub struct End;

    pub enum GameState {
        Wait,
        ChooseVictimCard,
        ChooseOneFromThree,
        ChooseTwoFromFour,
        End,
    }

    pub enum ActionKind {
        Safe,
        OnlyChallengeable,
        OnlyBlockable,
        Reactable,
    }

    pub trait Wait {
        fn with_count(count: usize) -> Wait;
        fn with_players(players: RawPlayers) -> Wait;
        fn info(&self) -> GameInfo<'_>;
        fn actions(&self) -> &PossibleActions;
        fn play(self, action: Action) -> ActionKind;
    }

    pub trait Safe {
        fn outcome(&self) -> Outcome;
        fn advance(self) -> GameState;
    }

    pub trait OnlyChallengeable {
        fn challenges(&self) -> &PossibleChallenges;
        fn challenge(self, challenge: action::Challenge) -> Challenge;
        fn outcome(&self) -> Outcome;
        fn advance(self) -> GameState;
    }

    pub trait OnlyBlockable {
        fn blocks(&self) -> &PossibleBlocks;
        fn block(self, block: action::Block) -> Block;
        fn outcome(&self) -> Outcome;
        fn advance(self) -> Wait;
    }

    pub trait Reactable {
        fn reactions(&self) -> &PossibleReactions;
        fn challenge(self, challenge: action::Challenge) -> Challenge;
        fn block(self, block: action::Block) -> Block;
        fn outcome(&self) -> Outcome;
        fn advance(self) -> GameState;
    }

    pub trait ChooseVictimCard {
        fn choices(&self) -> [Card; 2];
        fn advance(self, choice: Card) -> Wait;
    }

    pub trait ChooseOneFromThree {
        fn choices(&self) -> [Card; 3];
        fn advance(self, choice: Card) -> Wait;
    }

    pub trait ChooseTwoFromFour {
        fn choices(&self) -> [Card; 4];
        fn advance(self, choice: [Card; 2]) -> Wait;
    }

    pub trait Challenge {
        fn outcome(&self) -> Outcome;
        fn advance(self) -> GameState;
    }

    pub trait Block {
        fn challenges(&self) -> &PossibleChallenges;
        fn challenge(self, challenge: action::Challenge) -> Challenge;
        fn outcome(&self) -> Outcome;
        fn advance(self) -> Wait;
    }

    pub trait End {
        fn summary(self) -> Summary;
    }
}

impl CoupGame<ChooseVictimCard> {
    pub fn victim(&self) -> PlayerId {
        self.state.victim
    }
}

impl CoupGame<ChooseOneFromThree> {
    pub fn actor(&self) -> PlayerId {
        self.state.actor
    }
}

impl CoupGame<ChooseTwoFromFour> {
    pub fn actor(&self) -> PlayerId {
        self.state.actor
    }
}

impl<S: CoupGameState> CoupGame<S> {
    pub(crate) fn kill(mut self, victim: PlayerId) -> GameState {
        let coins = &mut self.data.coins;
        let players = &mut self.data.players;
        let player_coins = players.kill(victim).expect("Player should be killable");
        coins.return_coins(player_coins);

        if players.is_game_over() {
            GameState::End(CoupGame {
                data: self.data,
                state: End,
            })
        } else {
            GameState::Wait(self.end_turn())
        }
    }

    pub(crate) fn lose_influence(self, victim: PlayerId) -> GameState {
        let hand = self.data.players.hand_for(victim);
        match hand {
            Hand::Full(c1, c2) => GameState::ChooseVictimCard(CoupGame {
                data: self.data,
                state: ChooseVictimCard {
                    victim,
                    choices: [c1, c2],
                },
            }),
            Hand::Last(_, _) => self.kill(victim),
        }
    }

    pub(crate) fn withdraw(mut self, withdrawal: Withdrawal, actor: PlayerId) -> CoupGame<Wait> {
        let coins = self.data.players.get_coins_for(actor);
        let coins = self
            .data
            .coins
            .withdraw(withdrawal, coins)
            .expect("Should have coins left");
        self.data.players.set_coins_for(actor, coins);
        self.end_turn()
    }

    pub(crate) fn spend(&mut self, deposit: Deposit, actor: PlayerId) {
        let coins = self.data.players.get_coins_for(actor);
        let coins = self
            .data
            .coins
            .spend(deposit, coins)
            .expect("Should have coins left");
        self.data.players.set_coins_for(actor, coins);
    }

    pub(crate) fn end_turn(mut self) -> CoupGame<Wait> {
        self.data.players.end_turn();
        let possible_actions = self
            .data
            .players
            .generate_actions_for(self.data.players.current_player());

        self.transition_with_state(Wait { possible_actions })
    }

    pub(crate) fn transition_with_state<T: CoupGameState>(self, state: T) -> CoupGame<T> {
        CoupGame {
            data: self.data,
            state,
        }
    }

    pub(crate) fn transition_to_block(self, block: action::Block) -> CoupGame<Block> {
        let action::Block {
            actor,
            blocker,
            kind,
            ..
        } = block;

        let action = match kind {
            BlockableAct::ForeignAid => ChallengeableAct::BlockForeignAid,
            BlockableAct::Assassinate { .. } => ChallengeableAct::BlockAssassination,
            BlockableAct::Steal { claim, .. } => ChallengeableAct::BlockSteal { claim },
        };
        let possible_challenges = self
            .data
            .players
            .generate_challenges_against(blocker, action);

        self.transition_with_state(Block {
            possible_challenges,
            actor,
            blocker,
            kind,
        })
    }
}

// implementations of state machine traits
pub use game::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_game_info() {
        let players = RawPlayers::with_names(["Dave", "Garry"]).expect("Valid length");
        let game = CoupGame::with_players(players);

        let GameInfo {
            players,
            current_player,
            coins_remaining,
            deck,
        } = game.info();

        assert_eq!(coins_remaining, 46);
        assert_eq!(current_player, PlayerId::One);
        assert_eq!(players.alive().len(), 2);
        assert_eq!(deck.len(), 11);
    }
}
