#[allow(unused_imports)]
use rand::seq::SliceRandom;

use super::players::{Player, PlayerId};
use std::collections::HashMap;

type Map<PlayerId> = HashMap<PlayerId, (PlayerId, PlayerId)>;

#[derive(Debug)]
pub(super) struct CurrentPlayer {
    order: Map<PlayerId>,
    current_player: PlayerId,
}

impl CurrentPlayer {
    pub(crate) fn with_players(players: &HashMap<PlayerId, Player>) -> CurrentPlayer {
        let mut ids: Vec<PlayerId> = players.keys().copied().collect();
        #[cfg(test)]
        ids.sort_unstable();
        #[cfg(not(test))]
        ids.shuffle(&mut rand::thread_rng());

        let order: Map<PlayerId> = (0..ids.len())
            .map(|i| {
                let prev = ids[i.checked_sub(1).unwrap_or(ids.len() - 1)];
                let next = ids[(i + 1) % ids.len()];
                (ids[i], (prev, next))
            })
            .collect();

        let current_player = ids
            .first()
            .copied()
            .expect("Should always have at least 2 players");

        CurrentPlayer {
            order,
            current_player,
        }
    }

    pub(crate) fn current(&self) -> PlayerId {
        self.current_player
    }

    pub(crate) fn end_turn(&mut self) {
        if !self.order.is_empty() {
            let (_, next) = self.order[&self.current_player];
            self.current_player = next;
        }
    }

    pub(crate) fn order(&self) -> impl Iterator<Item = PlayerId> + use<'_> {
        std::iter::successors(Some(self.current_player), |cur| {
            self.order.get(cur).map(|(_, next)| next).copied()
        })
    }

    pub(crate) fn kill(&mut self, id: PlayerId) {
        if !self.order.is_empty() {
            let (prev, next) = self.order[&id];
            if self.current_player == id {
                self.current_player = next;
            }

            self.order.remove(&id);

            // update links
            let cur = prev;
            if let Some((prev, _)) = self.order.get(&cur) {
                self.order.insert(cur, (*prev, next));
            }

            if let Some((_, n_next)) = self.order.get(&next) {
                self.order.insert(next, (cur, *n_next));
            }
        }
    }
}
