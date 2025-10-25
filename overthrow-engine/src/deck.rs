// TODO: once this is stabilized, we can remove this crate
#![allow(unstable_name_collisions)]
use std::fmt::Display;

use itermore::IterArrayChunks;
use rand::seq::SliceRandom;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use subenum::subenum;

// standard starting deck
const STARTING_DECK: [Card; 15] = [
    Card::Ambassador,
    Card::Ambassador,
    Card::Ambassador,
    Card::Assassin,
    Card::Assassin,
    Card::Assassin,
    Card::Captain,
    Card::Captain,
    Card::Captain,
    Card::Contessa,
    Card::Contessa,
    Card::Contessa,
    Card::Duke,
    Card::Duke,
    Card::Duke,
];

#[subenum(BlockStealClaim)]
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Card {
    #[subenum(BlockStealClaim)]
    Ambassador,
    Contessa,
    Assassin,
    Duke,
    #[subenum(BlockStealClaim)]
    Captain,
}

use std::fmt;
impl Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Card::Ambassador => write!(f, "Ambassador"),
            Card::Contessa => write!(f, "Contessa"),
            Card::Assassin => write!(f, "Assassin"),
            Card::Duke => write!(f, "Duke"),
            Card::Captain => write!(f, "Captain"),
        }
    }
}

impl From<&BlockStealClaim> for Card {
    fn from(value: &BlockStealClaim) -> Self {
        match value {
            BlockStealClaim::Ambassador => Card::Ambassador,
            BlockStealClaim::Captain => Card::Captain,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Hand {
    Full(Card, Card),
    Last(Card, DeadCard),
}

impl Hand {
    pub(crate) fn has_card(&self, card: Card) -> bool {
        match self {
            Hand::Full(c1, c2) => *c1 == card || *c2 == card,
            Hand::Last(c1, _) => *c1 == card,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(transparent)]
pub struct DeadCard(pub(crate) Card);

impl DeadCard {
    pub fn card(&self) -> Card {
        self.0
    }
}

#[derive(Debug)]
pub(crate) struct DeadHand(pub(crate) DeadCard, pub(crate) DeadCard);

#[derive(Debug, Clone)]
pub struct Deck {
    deck: Vec<Card>,
}

impl Deck {
    pub(crate) fn with_count(player_count: u8) -> (Deck, Vec<Hand>) {
        let mut deck: Vec<Card> = STARTING_DECK.into();
        deck.shuffle(&mut rand::thread_rng());

        let cards_left = deck.len() - (2 * player_count) as usize;

        let hands = deck
            .drain(cards_left..)
            .array_chunks()
            .map(|[c1, c2]| Hand::Full(c1, c2))
            .collect();

        (Deck { deck }, hands)
    }

    pub(crate) fn shuffle(&mut self) {
        self.deck.shuffle(&mut rand::thread_rng());
    }

    // cards remaining in pile
    pub(crate) fn cards(&self) -> &[Card] {
        &self.deck
    }

    pub(crate) fn draw_two(&mut self) -> [Card; 2] {
        [self.deck.pop(), self.deck.pop()].map(|card| card.expect("Deck should have cards left"))
    }

    pub(crate) fn return_cards(&mut self, cards: &[Card]) {
        self.deck.extend_from_slice(cards);
        self.shuffle();
    }
}
