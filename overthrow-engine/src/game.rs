use super::action::{
    self, BlockableAct, OnlyChallengeableAct, PossibleActions, PossibleBlocks, PossibleChallenges,
    PossibleReactions, ReactableAct, SafeAct,
};
use super::coins::{Deposit, Withdrawal};
use super::deck::{DeadCard, Hand};

use super::action::{Act, Action};
use super::coins::CoinPile;
use super::deck::{Card, Deck};
use super::machine::*;
use super::players::{Player, PlayerId, Players, RawPlayers};
use itertools::{Itertools, izip};

impl WaitState for CoupGame<Wait> {
    fn with_count(count: usize) -> CoupGame<Wait> {
        assert!((2..=6).contains(&count));
        let players = vec![String::new(); count];
        let count = count as u8;
        let (deck, hands) = Deck::with_count(count);
        let (coins, player_coins) = CoinPile::with_count(count);

        // compile initial player data
        let data = izip!(players, player_coins, hands)
            .map(|(name, coins, hand)| Player::new(name, coins, hand));
        let players = Players::with_players(PlayerId::iter().zip(data));
        let data = Box::new(CoupData {
            players,
            coins,
            deck,
        });

        let possible_actions = data
            .players
            .generate_actions_for(data.players.current_player());

        CoupGame {
            data,
            state: Wait { possible_actions },
        }
    }

    fn with_players(players: RawPlayers) -> CoupGame<Wait> {
        let RawPlayers(players, player_count) = players;

        let (deck, hands) = Deck::with_count(player_count);
        let (coins, player_coins) = CoinPile::with_count(player_count);

        // compile initial player data
        let data = izip!(players, player_coins, hands)
            .map(|(name, coins, hand)| Player::new(name, coins, hand));
        let players = Players::with_players(PlayerId::iter().zip(data));
        let data = Box::new(CoupData {
            players,
            coins,
            deck,
        });

        let possible_actions = data
            .players
            .generate_actions_for(data.players.current_player());

        CoupGame {
            data,
            state: Wait { possible_actions },
        }
    }

    fn info(&self) -> GameInfo<'_> {
        GameInfo {
            players: &self.data.players,
            current_player: self.data.players.current_player(),
            coins_remaining: self.data.coins.remaining(),
            deck: self.data.deck.cards(),
        }
    }

    fn actions(&self) -> &PossibleActions {
        &self.state.possible_actions
    }

    fn play(mut self, action: Action) -> ActionKind {
        let players = &mut self.data.players;
        let actor = action.actor;
        match action.kind {
            Act::Assassinate { victim } => {
                let kind = ReactableAct::Assassinate { victim };
                let possible_reactions = players.generate_reactions_against(actor, &kind);
                ActionKind::Reactable(self.transition_with_state(Reactable {
                    actor,
                    kind,
                    possible_reactions,
                }))
            }
            Act::Income => ActionKind::Safe(self.transition_with_state(Safe {
                actor,
                kind: SafeAct::Income,
            })),
            Act::Coup { victim } => ActionKind::Safe(self.transition_with_state(Safe {
                actor,
                kind: SafeAct::Coup { victim },
            })),
            Act::Exchange => {
                let kind = OnlyChallengeableAct::Exchange;
                let possible_challenges = players.generate_challenges_against(actor, kind.into());

                ActionKind::OnlyChallengeable(self.transition_with_state(OnlyChallengeable {
                    actor,
                    kind,
                    possible_challenges,
                }))
            }
            Act::ForeignAid => {
                let possible_blocks = players.generate_blocks_against(actor);
                ActionKind::OnlyBlockable(self.transition_with_state(OnlyBlockable {
                    actor,
                    possible_blocks,
                }))
            }
            Act::Steal { victim } => {
                let kind = ReactableAct::Steal { victim };
                let possible_reactions = players.generate_reactions_against(actor, &kind);

                ActionKind::Reactable(self.transition_with_state(Reactable {
                    actor,
                    kind,
                    possible_reactions,
                }))
            }
            Act::Tax => {
                let kind = OnlyChallengeableAct::Tax;
                let possible_challenges = players.generate_challenges_against(actor, kind.into());
                ActionKind::OnlyChallengeable(self.transition_with_state(OnlyChallengeable {
                    actor,
                    kind,
                    possible_challenges,
                }))
            }
        }
    }
}

impl ReactableState for CoupGame<Reactable> {
    fn reactions(&self) -> &PossibleReactions {
        &self.state.possible_reactions
    }

    fn block(self, block: action::Block) -> CoupGame<Block> {
        self.transition_to_block(block)
    }

    fn challenge(self, challenge: action::Challenge) -> CoupGame<Challenge> {
        let action::Challenge {
            actor,
            challenger,
            kind,
        } = challenge;

        CoupGame {
            data: self.data,
            state: Challenge {
                actor,
                challenger,
                kind,
            },
        }
    }

    fn outcome(&self) -> Outcome {
        match self.state.kind {
            ReactableAct::Steal { victim } => Outcome::LoseCoins {
                actor: victim,
                amount: 2,
            },
            ReactableAct::Assassinate { victim } => Outcome::LosesInfluence { victim },
        }
    }

    fn advance(mut self) -> GameState {
        match self.state.kind {
            ReactableAct::Assassinate { victim } => {
                let actor = self.state.actor;
                self.spend(Deposit::Assassinate, actor);
                self.lose_influence(victim)
            }
            ReactableAct::Steal { victim } => {
                let victim_coins = self.data.players.get_coins_for(victim);
                let actor_coins = self.data.players.get_coins_for(self.state.actor);
                let (victim_coins, actor_coins) = victim_coins.steal(actor_coins);
                self.data.players.set_coins_for(victim, victim_coins);
                self.data
                    .players
                    .set_coins_for(self.state.actor, actor_coins);

                GameState::Wait(self.end_turn())
            }
        }
    }
}

impl SafeState for CoupGame<Safe> {
    fn outcome(&self) -> Outcome {
        match self.state.kind {
            SafeAct::Income => Outcome::GainCoins {
                actor: self.state.actor,
                amount: 1,
            },
            SafeAct::Coup { victim } => Outcome::LosesInfluence { victim },
        }
    }

    fn advance(mut self) -> GameState {
        match self.state.kind {
            SafeAct::Income => {
                let actor = self.state.actor;
                GameState::Wait(self.withdraw(Withdrawal::Income, actor))
            }
            SafeAct::Coup { victim } => {
                self.spend(Deposit::Coup, self.state.actor);
                self.lose_influence(victim)
            }
        }
    }
}

impl OnlyChallengeableState for CoupGame<OnlyChallengeable> {
    fn challenges(&self) -> &PossibleChallenges {
        &self.state.possible_challenges
    }

    fn challenge(self, challenge: action::Challenge) -> CoupGame<Challenge> {
        let action::Challenge {
            actor,
            challenger,
            kind,
            ..
        } = challenge;

        CoupGame {
            data: self.data,
            state: Challenge {
                actor,
                challenger,
                kind,
            },
        }
    }

    fn outcome(&self) -> Outcome {
        match self.state.kind {
            OnlyChallengeableAct::Exchange => Outcome::ExchangesCards {
                actor: self.state.actor,
            },
            OnlyChallengeableAct::Tax => Outcome::GainCoins {
                actor: self.state.actor,
                amount: 3,
            },
        }
    }

    fn advance(mut self) -> GameState {
        match self.state.kind {
            OnlyChallengeableAct::Exchange => {
                let hand = self.data.players.hand_for(self.state.actor);
                let [c1, c2] = self.data.deck.draw_two();
                match hand {
                    Hand::Full(c3, c4) => GameState::ChooseTwoFromFour(CoupGame {
                        data: self.data,
                        state: ChooseTwoFromFour {
                            actor: self.state.actor,
                            choices: [c1, c2, c3, c4],
                        },
                    }),
                    Hand::Last(c3, _) => GameState::ChooseOneFromThree(CoupGame {
                        data: self.data,
                        state: ChooseOneFromThree {
                            actor: self.state.actor,
                            choices: [c1, c2, c3],
                        },
                    }),
                }
            }
            OnlyChallengeableAct::Tax => {
                let actor = self.state.actor;
                GameState::Wait(self.withdraw(Withdrawal::Tax, actor))
            }
        }
    }
}

impl OnlyBlockableState for CoupGame<OnlyBlockable> {
    fn blocks(&self) -> &PossibleBlocks {
        &self.state.possible_blocks
    }

    fn block(self, block: action::Block) -> CoupGame<Block> {
        self.transition_to_block(block)
    }

    fn outcome(&self) -> Outcome {
        Outcome::GainCoins {
            actor: self.state.actor,
            amount: 2,
        }
    }

    fn advance(self) -> CoupGame<Wait> {
        let actor = self.state.actor;
        self.withdraw(Withdrawal::ForeignAid, actor)
    }
}

impl ChooseVictimCardState for CoupGame<ChooseVictimCard> {
    fn choices(&self) -> [Card; 2] {
        self.state.choices
    }

    fn advance(mut self, choice: Card) -> CoupGame<Wait> {
        let hand = self.data.players.hand_for(self.state.victim);
        let Hand::Full(c1, c2) = hand else {
            panic!("Must have two cards")
        };

        let remaining_card = if c1 == choice {
            c2
        } else if c2 == choice {
            c1
        } else {
            panic!("Choice must be valid")
        };

        let hand = Hand::Last(remaining_card, DeadCard(choice));
        self.data.players.exchange(self.state.victim, hand);

        self.end_turn()
    }
}

impl ChooseOneFromThreeState for CoupGame<ChooseOneFromThree> {
    fn choices(&self) -> [Card; 3] {
        self.state.choices
    }

    fn advance(mut self, choice: Card) -> CoupGame<Wait> {
        let Some((index, _)) = self
            .state
            .choices
            .into_iter()
            .enumerate()
            .find(|(_, c)| *c == choice)
        else {
            panic!("Invalid choice provided: {:?}", choice);
        };

        let Hand::Last(_, dead) = self.data.players.hand_for(self.state.actor) else {
            panic!("Must be on last card")
        };
        let hand = Hand::Last(choice, dead);
        self.data.players.exchange(self.state.actor, hand);

        // getting other two cards to return them to the deck
        let other_cards: [Card; 2] = self
            .state
            .choices
            .into_iter()
            .enumerate()
            .filter_map(|(i, card)| (index != i).then_some(card))
            .collect_array()
            .expect("Two other cards must exist");
        self.data.deck.return_cards(&other_cards);

        self.end_turn()
    }
}

impl ChooseTwoFromFourState for CoupGame<ChooseTwoFromFour> {
    fn choices(&self) -> [Card; 4] {
        self.state.choices
    }

    fn advance(mut self, [c1, c2]: [Card; 2]) -> CoupGame<Wait> {
        let choices = self.state.choices;

        // find indices of chosen cards in our choices array (if they exist)
        // this is for later when we want to return the correct cards to our
        // deck
        let indices =
            choices
                .into_iter()
                .enumerate()
                .fold([None, None], |[i1, i2], (index, card)| {
                    let c1_index = (c1 == card && i1.is_none()).then_some(index);
                    let c2_index = (c2 == card).then_some(index);
                    let c1_xor_c2 = c1_index.xor(c2_index);

                    [i1.or(c1_index), i2.or(c1_xor_c2.and(c2_index))]
                });

        let [Some(i1), Some(i2)] = indices else {
            panic!("Choices were not valid: {:?}", [c1, c2]);
        };

        let hand = Hand::Full(c1, c2);
        self.data.players.exchange(self.state.actor, hand);

        let remaining_cards: [Card; 2] = choices
            .into_iter()
            .enumerate()
            .filter_map(|(index, card)| (index != i1 && index != i2).then_some(card))
            .collect_array()
            .expect("Must have two cards left");
        self.data.deck.return_cards(&remaining_cards);

        self.end_turn()
    }
}

impl ChallengeState for CoupGame<Challenge> {
    fn outcome(&self) -> Outcome {
        let claim = (&self.state.kind).into();
        let victim =
            self.data
                .players
                .challenge_winner(self.state.actor, self.state.challenger, claim);

        Outcome::LosesInfluence { victim }
    }

    fn advance(self) -> GameState {
        let claim = (&self.state.kind).into();
        let victim =
            self.data
                .players
                .challenge_winner(self.state.actor, self.state.challenger, claim);

        self.lose_influence(victim)
    }
}

impl BlockState for CoupGame<Block> {
    fn challenges(&self) -> &PossibleChallenges {
        &self.state.possible_challenges
    }

    fn challenge(self, challenge: action::Challenge) -> CoupGame<Challenge> {
        let action::Challenge {
            actor,
            challenger,
            kind,
            ..
        } = challenge;

        CoupGame {
            data: self.data,
            state: Challenge {
                actor,
                challenger,
                kind,
            },
        }
    }

    fn outcome(&self) -> Outcome {
        match self.state.kind {
            BlockableAct::ForeignAid => Outcome::LoseTurn {
                victim: self.state.actor,
            },
            BlockableAct::Steal { .. } => Outcome::LoseTurn {
                victim: self.state.actor,
            },
            BlockableAct::Assassinate { .. } => Outcome::LoseCoins {
                actor: self.state.actor,
                amount: 3,
            },
        }
    }

    fn advance(mut self) -> CoupGame<Wait> {
        if matches!(self.state.kind, BlockableAct::Assassinate { .. }) {
            let actor = self.state.actor;
            self.spend(Deposit::Assassinate, actor);
        }

        self.end_turn()
    }
}

impl EndState for CoupGame<End> {
    fn summary(self) -> Summary {
        assert_eq!(self.data.players.alive().len(), 1);
        let last_player = self
            .data
            .players
            .alive()
            .keys()
            .copied()
            .next()
            .expect("Last man standing");

        Summary {
            winner: last_player,
        }
    }
}
