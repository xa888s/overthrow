use jiff::Timestamp;
use overthrow_engine::deck::Hand;
pub use overthrow_engine::{
    action::{Action, Block, Blocks, Challenge, Reaction},
    deck::Card,
    machine::{Outcome, Summary},
    players::PlayerId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

// TODO: remove redundant information from messages to simplify schema
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientMessage {
    GameId(Uuid),
    PlayerId(PlayerId),
    Info(Info),
    End(Summary),
    GameCancelled,
    Outcome(Outcome),
    ActionChoices(Vec<Action>),
    ChallengeChoice(Challenge, Timestamp),
    BlockChoices(Blocks, Timestamp),
    ReactionChoices(Vec<Reaction>, Timestamp),
    VictimChoices([Card; 2]),
    OneFromThreeChoices([Card; 3]),
    TwoFromFourChoices([Card; 4]),
}

// TODO: remove redundant information from responses to simplify schema
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientResponse {
    Pass,
    Block(Card),
    Challenge,
    Act(Action),
    React(Reaction),
    ChooseVictim(Card),
    ExchangeOne(Card),
    ExchangeTwo([Card; 2]),
}

#[derive(Debug, Clone, Error, Deserialize, Serialize, JsonSchema)]
pub enum ClientError {
    #[error("Received message from client before it was expected")]
    NotReady,
    #[error("Response from client is not in the correct format, or does not contain valid values")]
    InvalidResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum PlayerView {
    Other {
        name: String,
        coins: u8,
        revealed_cards: Vec<Card>,
    },
    Me {
        name: String,
        coins: u8,
        hand: Hand,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Info {
    pub player_views: HashMap<PlayerId, PlayerView>,
    pub current_player: PlayerId,
    pub coins_remaining: u8,
}
