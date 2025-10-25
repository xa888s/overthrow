pub use overthrow_engine::{
    action::{Action, Blocks, Challenge, Reaction},
    deck::Card,
    machine::{Outcome, Summary},
    players::PlayerId,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    ChallengeChoice(Challenge),
    BlockChoices(Blocks),
    ReactionChoices(Vec<Reaction>),
    VictimChoices([Card; 2]),
    OneFromThreeChoices([Card; 3]),
    TwoFromFourChoices([Card; 4]),
}

// TODO: remove redundant information from responses to simplify schema
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientResponse {
    Block(Card),
    Challenge(bool),
    Act(Action),
    React(Reaction),
    ChooseVictim(Card),
    ExchangeOne(Card),
    ExchangeTwo([Card; 2]),
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub enum ClientError {
    NotReady,
    InvalidResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlayerView {
    pub name: String,
    pub coins: u8,
    pub revealed_cards: Vec<Card>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Info {
    pub player_views: HashMap<PlayerId, PlayerView>,
    pub current_player: PlayerId,
    pub coins_remaining: u8,
}
