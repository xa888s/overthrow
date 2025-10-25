use std::ops::Index;

use crate::{coins::PlayerCoins, deck::Hand, players::PlayerId};
use arrayvec::ArrayVec;

const MAX_PLAYER_COUNT: usize = 6;

#[derive(Debug)]
pub enum Player {
    Alive {
        name: String,
        coins: PlayerCoins,
        hand: Hand,
    },
    Dead {
        name: String,
        hand: Hand,
    },
}

#[derive(Debug)]
pub struct PlayerMap {
    players: ArrayVec<Player, MAX_PLAYER_COUNT>,
}

impl PlayerMap {
    pub fn new(player_count: usize) -> PlayerMap {
        assert!((2..=6).contains(&player_count));
        let players = ArrayVec::new();
        PlayerMap { players }
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
