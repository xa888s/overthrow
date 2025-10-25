use super::deck::BlockStealClaim;
use super::deck::Card;
use super::players::PlayerId;
use itertools::chain;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Display;
use subenum::subenum;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Reaction {
    Challenge(Challenge),
    Block(Block),
}

impl Reaction {
    pub fn reactor(&self) -> PlayerId {
        let (Reaction::Challenge(Challenge {
            challenger: reactor,
            ..
        })
        | Reaction::Block(Block {
            blocker: reactor, ..
        })) = self;

        *reactor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct Action {
    pub(crate) actor: PlayerId,
    pub(crate) kind: Act,
}

impl Action {
    pub(crate) fn new(actor: PlayerId, kind: Act) -> Action {
        Action { actor, kind }
    }

    pub fn actor(&self) -> PlayerId {
        self.actor
    }

    pub fn kind(&self) -> Act {
        self.kind
    }

    pub fn claim(&self) -> Option<Card> {
        self.kind.claim()
    }
}

#[subenum(OnlyBlockableAct, OnlyChallengeableAct, ReactableAct, SafeAct)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub enum Act {
    #[subenum(SafeAct)]
    Income,
    #[subenum(OnlyBlockableAct)]
    ForeignAid,
    #[subenum(OnlyChallengeableAct)]
    Tax,
    #[subenum(OnlyChallengeableAct)]
    Exchange,
    #[subenum(ReactableAct)]
    Steal { victim: PlayerId },
    #[subenum(ReactableAct)]
    Assassinate { victim: PlayerId },
    #[subenum(SafeAct)]
    Coup { victim: PlayerId },
}

use std::fmt;
impl Display for Act {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Act::Income => write!(f, "Take Income"),
            Act::ForeignAid => write!(f, "Take Foreign Aid"),
            Act::Tax => write!(f, "Take Tax"),
            Act::Exchange => write!(f, "Exchange Cards"),
            Act::Steal { victim } => write!(f, "Take Coins From Player {victim}"),
            Act::Assassinate { victim } => write!(f, "Assasinate Player {victim}"),
            Act::Coup { victim } => write!(f, "Coup Player {victim}"),
        }
    }
}

impl Act {
    pub fn claim(&self) -> Option<Card> {
        match self {
            Act::Income => None,
            Act::ForeignAid => None,
            Act::Tax => Some(Card::Duke),
            Act::Exchange => Some(Card::Ambassador),
            Act::Steal { .. } => Some(Card::Captain),
            Act::Assassinate { .. } => Some(Card::Assassin),
            Act::Coup { .. } => None,
        }
    }
}

impl From<&ReactableAct> for ChallengeableAct {
    fn from(value: &ReactableAct) -> Self {
        match value {
            ReactableAct::Steal { victim } => ChallengeableAct::Steal { victim: *victim },
            ReactableAct::Assassinate { victim } => {
                ChallengeableAct::Assassinate { victim: *victim }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum BlockableAct {
    ForeignAid,
    Steal {
        victim: PlayerId,
        claim: BlockStealClaim,
    },
    Assassinate {
        victim: PlayerId,
    },
}

impl Display for BlockableAct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockableAct::ForeignAid => write!(f, "Take Foreign Aid"),
            BlockableAct::Steal { victim, claim } => {
                let claim: Card = claim.into();
                write!(f, "Steal From {victim} As {claim}")
            }
            BlockableAct::Assassinate { victim } => write!(f, "Assasinate {victim}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub enum Blocks {
    Other(Block),
    Steal(Block, Block),
}

impl Blocks {
    pub fn blocker_id(&self) -> PlayerId {
        let (Blocks::Other(block) | Blocks::Steal(block, _)) = self;
        block.blocker
    }

    pub fn claims(&self, card: Card) -> bool {
        match self {
            Blocks::Other(b1) => b1.claim() == card,
            Blocks::Steal(b1, b2) => b1.claim() == card || b2.claim() == card,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum ChallengeableAct {
    Exchange,
    Tax,
    Steal { victim: PlayerId },
    Assassinate { victim: PlayerId },
    BlockAssassination,
    BlockForeignAid,
    BlockSteal { claim: BlockStealClaim },
}

impl Display for ChallengeableAct {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChallengeableAct::Tax => write!(f, "Take Tax"),
            ChallengeableAct::Exchange => write!(f, "Exchange Cards"),
            ChallengeableAct::Steal { victim } => write!(f, "Take Coins From Player {victim}"),
            ChallengeableAct::Assassinate { victim } => write!(f, "Assasinate Player {victim}"),
            ChallengeableAct::BlockAssassination => write!(f, "Block Assasination"),
            ChallengeableAct::BlockForeignAid => write!(f, "Block Foreign Aid"),
            ChallengeableAct::BlockSteal { claim } => {
                let claim: Card = claim.into();
                write!(f, "Block Steal as {claim:?}")
            }
        }
    }
}

impl From<&ChallengeableAct> for Card {
    fn from(value: &ChallengeableAct) -> Self {
        match value {
            ChallengeableAct::Assassinate { .. } => Card::Assassin,
            ChallengeableAct::Exchange => Card::Ambassador,
            ChallengeableAct::Tax => Card::Duke,
            ChallengeableAct::Steal { .. } => Card::Captain,
            ChallengeableAct::BlockAssassination => Card::Contessa,
            ChallengeableAct::BlockForeignAid => Card::Duke,
            ChallengeableAct::BlockSteal { claim } => claim.into(),
        }
    }
}

impl From<OnlyChallengeableAct> for ChallengeableAct {
    fn from(value: OnlyChallengeableAct) -> Self {
        match value {
            OnlyChallengeableAct::Exchange => ChallengeableAct::Exchange,
            OnlyChallengeableAct::Tax => ChallengeableAct::Tax,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Block {
    pub(crate) actor: PlayerId,
    pub(crate) blocker: PlayerId,
    pub(crate) kind: BlockableAct,
}

impl Block {
    pub fn actor(&self) -> PlayerId {
        self.actor
    }

    pub fn blocker(&self) -> PlayerId {
        self.blocker
    }

    pub fn kind(&self) -> &BlockableAct {
        &self.kind
    }

    pub fn claim(&self) -> Card {
        match self.kind {
            BlockableAct::ForeignAid => Card::Duke,
            BlockableAct::Steal { claim, .. } => {
                if matches!(claim, BlockStealClaim::Ambassador) {
                    Card::Ambassador
                } else {
                    Card::Captain
                }
            }
            BlockableAct::Assassinate { .. } => Card::Assassin,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct PossibleActions {
    pub(crate) current_player: PlayerId,
    pub(crate) assassinations: Vec<Action>,
    pub(crate) coups: Vec<Action>,
    pub(crate) steal: Vec<Action>,
    pub(crate) basic: Vec<Action>,
}

impl PossibleActions {
    pub fn assassinations(&self) -> &[Action] {
        &self.assassinations
    }

    pub fn coups(&self) -> &[Action] {
        &self.coups
    }

    pub fn steal(&self) -> &[Action] {
        &self.steal
    }

    pub fn basic(&self) -> &[Action] {
        &self.basic
    }

    pub fn all(&self) -> impl Iterator<Item = &Action> {
        chain!(&self.assassinations, &self.coups, &self.steal, &self.basic)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Challenge {
    pub(crate) actor: PlayerId,
    pub(crate) challenger: PlayerId,
    pub(crate) kind: ChallengeableAct,
}

impl Challenge {
    pub fn challenger(&self) -> PlayerId {
        self.challenger
    }

    pub fn kind(&self) -> &ChallengeableAct {
        &self.kind
    }

    pub fn actor(&self) -> PlayerId {
        self.actor
    }
}

#[derive(Debug)]
pub struct PossibleReactions {
    pub(crate) block: Blocks,
    pub(crate) challenge: HashMap<PlayerId, Challenge>,
}

impl PossibleReactions {
    pub fn block(&self) -> &Blocks {
        &self.block
    }

    pub fn challenges(&self) -> &HashMap<PlayerId, Challenge> {
        &self.challenge
    }

    pub fn all(&self) -> HashMap<PlayerId, Vec<Reaction>> {
        let mut map: HashMap<PlayerId, Vec<Reaction>> =
            HashMap::with_capacity(self.challenge.len() + 2);

        match self.block.clone() {
            Blocks::Other(block) => {
                map.insert(block.blocker(), vec![Reaction::Block(block)]);
            }
            Blocks::Steal(b1, b2) => {
                map.insert(b1.blocker(), vec![Reaction::Block(b1)]);
                map.insert(b2.blocker(), vec![Reaction::Block(b2)]);
            }
        };

        for challenge in self.challenge.values().cloned() {
            let challenger = challenge.challenger();
            let reaction = Reaction::Challenge(challenge);
            map.entry(challenger)
                .and_modify(|reactions| reactions.push(reaction.clone()))
                .or_insert_with(|| vec![reaction]);
        }

        map
    }
}

#[derive(Debug)]
pub struct PossibleBlocks {
    pub(crate) blocks: HashMap<PlayerId, Block>,
}

impl PossibleBlocks {
    pub fn all(&self) -> &HashMap<PlayerId, Block> {
        &self.blocks
    }
}

#[derive(Debug)]
pub struct PossibleChallenges {
    pub(crate) challenges: HashMap<PlayerId, Challenge>,
}

impl PossibleChallenges {
    pub fn all(&self) -> &HashMap<PlayerId, Challenge> {
        &self.challenges
    }
}
