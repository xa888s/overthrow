use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt::Display;

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

use std::fmt;
impl Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", *self as u8)
    }
}

impl PlayerId {
    pub fn iter() -> impl Iterator<Item = PlayerId> {
        use PlayerId::*;
        [One, Two, Three, Four, Five, Six].into_iter()
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        action::{Act, Action, PossibleActions},
        machine::{CoupGame, Wait, WaitState},
    };
    use pretty_assertions::assert_eq;
    use std::sync::LazyLock;

    use super::*;
    static BASIC_GAME: LazyLock<CoupGame<Wait>> =
        LazyLock::new(|| CoupGame::with_player_names(["Dave", "Garry"]));

    #[test]
    fn basic_generate_actions() {
        // start basic game with two players
        let game = &*BASIC_GAME;
        let actions = game.actions();

        let player_one_actions = PossibleActions {
            actor: PlayerId::One,
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
