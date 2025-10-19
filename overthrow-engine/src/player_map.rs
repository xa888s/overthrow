use arrayvec::ArrayVec;
use std::ops::Deref;

const MAX_PLAYER_COUNT: usize = 6;

#[derive(Debug)]
struct PlayerData;

#[derive(Debug)]
pub enum Player {
    Alive(PlayerData),
    Dead(PlayerData),
}

#[derive(Debug)]
pub struct PlayerMap {
    players: ArrayVec<Player, MAX_PLAYER_COUNT>,
}
