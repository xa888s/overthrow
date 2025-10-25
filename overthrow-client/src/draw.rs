use itertools::Itertools;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph},
};
use tui_big_text::{BigText, PixelSize};

use crate::tui::State;

pub fn game_view(state: &State, f: &mut Frame) {
    if let State::InGame {
        game_id,
        player_id,
        info,
    } = state
    {
        // split screen into 2/3 for player views, and 1/3 for input box
        let layout = Layout::vertical([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]);
        let area = f.area();
        let [info_area, input_area] = layout.split(area)[..]
            .try_into()
            .expect("Two constraints provided");

        // have each player view take an equal amount of space
        let player_count = info.player_views.len();
        let view_constraints = (0..player_count).map(|_| Constraint::Ratio(1, player_count as u32));
        let layout = Layout::horizontal(view_constraints);

        // sort views by player id
        let sorted_views = info.player_views.iter().sorted_by_key(|(id, _)| **id);

        let player_info_areas = layout.split(info_area);
        for (area, (id, view)) in player_info_areas.iter().zip(sorted_views) {
            let num_id = *id as u8;
            let title = Line::from(format!("Player {num_id}"));
            let title = if id == player_id {
                title.underlined()
            } else {
                title
            };
            let block = Block::bordered().title_top(title);
            let view = format!(
                "Name: {}\nCoins: {}\nCards: {:?}",
                view.name, view.coins, view.revealed_cards
            );
            let paragraph = Paragraph::new(view).block(block);
            f.render_widget(paragraph, *area);
        }

        f.render_widget(
            Block::bordered().title(format!("Input (Game ID: {game_id})")),
            input_area,
        );
    }
}

pub fn splash_page(state: &State, f: &mut Frame) {
    if matches!(state, State::Connecting | State::InLobby { .. }) {
        let logo = BigText::builder()
            .pixel_size(PixelSize::Full)
            .lines(["overthrow".into()])
            .centered()
            .build();

        // centre logo within the buffer
        const LOGO_HEIGHT: u16 = 7;
        let area = f.area();
        let y = (area.height / 2 - LOGO_HEIGHT / 2) - 2;
        let area = Rect::new(area.x, y, area.width, LOGO_HEIGHT + 3);
        let layout = Layout::vertical([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ]);
        let [logo_area, game_id_area, loading] = layout.split(area)[..]
            .try_into()
            .expect("Specified 3 constraints");

        let loading_text = Paragraph::new("Waiting for game to start...").centered();

        f.render_widget(logo, logo_area);

        if let State::InLobby { game_id } = state {
            let game_id_text = Paragraph::new(format!("Game ID: {game_id}")).centered();
            f.render_widget(game_id_text, game_id_area);
            f.render_widget(loading_text, loading);
        } else {
            f.render_widget(loading_text, game_id_area);
        }
    }
}
