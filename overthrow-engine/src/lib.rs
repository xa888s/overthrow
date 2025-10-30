#![warn(unused_crate_dependencies)]
#![feature(array_try_map)]
pub mod action;
mod coins;
mod current_player;
pub mod deck;
mod game;
pub use game::match_to_indices;
pub mod machine;
pub mod player_map;
pub mod players;
