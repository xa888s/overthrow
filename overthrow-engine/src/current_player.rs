use arrayvec::ArrayVec;
#[allow(unused_imports)]
use rand::seq::SliceRandom;

use crate::player_map::MAX_PLAYER_COUNT;

use super::players::PlayerId;

#[derive(Debug)]
pub(super) struct CurrentPlayer {
    order: ArrayVec<PlayerId, MAX_PLAYER_COUNT>,
    current: usize,
}

impl CurrentPlayer {
    pub(crate) fn new(player_count: usize) -> CurrentPlayer {
        let mut order: ArrayVec<_, _> = PlayerId::iter().take(player_count).collect();
        order[..].shuffle(&mut rand::thread_rng());

        CurrentPlayer { order, current: 0 }
    }

    pub(crate) fn current(&self) -> PlayerId {
        self.order[self.current]
    }

    pub(crate) fn end_turn(&mut self) {
        self.current = (self.current + 1) % self.order.len();
    }

    pub(crate) fn order(&self) -> impl Iterator<Item = PlayerId> {
        self.order.iter().copied()
    }

    pub(crate) fn kill(&mut self, player_id: PlayerId) {
        let index = self
            .order
            .iter()
            .enumerate()
            .find_map(|(index, id)| (player_id == *id).then_some(index))
            .expect("Player ID should be valid");

        let current_player = self.current();

        // killed player won't have another turn, so it'll go to next player
        if current_player == player_id {
            self.end_turn();
        }
        self.order.remove(index);

        // now we have to fix the index
        self.current = self
            .order
            .iter()
            .enumerate()
            .find_map(|(index, id)| (*id == current_player).then_some(index))
            .expect("Player ID should be valid");
    }
}
