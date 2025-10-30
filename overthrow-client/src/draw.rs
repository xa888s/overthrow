use itertools::Itertools;
use overthrow_types::{Info, PlayerId, PlayerView};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Stylize,
    text::Line,
    widgets::{Block, List, Paragraph},
};
use tui_big_text::{BigText, PixelSize};
use uuid::Uuid;

use crate::tui::{State, UiState};

fn draw_info_view(player_id: PlayerId, info: &Info, area: Rect, f: &mut Frame) {
    // have each player view take an equal amount of space
    let player_count = info.player_views.len();
    let view_constraints = (0..player_count).map(|_| Constraint::Ratio(1, player_count as u32));
    let layout = Layout::horizontal(view_constraints);

    // sort views besides local player by player id
    let sorted_views = info
        .player_views
        .iter()
        .filter(|(id, _)| **id != player_id)
        .sorted_by_key(|(id, _)| **id);

    // local player should always be first (leftmost in UI)
    let local_player_view = info
        .player_views
        .get_key_value(&player_id)
        .expect("We are a player aren't we?");

    use std::iter::once;
    let player_views = once(local_player_view).chain(sorted_views);
    let player_info_areas = layout.split(area);

    // create blocks for each player view
    for (area, (id, view)) in player_info_areas.iter().zip(player_views) {
        let title = Line::from(format!("Player {id}"));
        let (view, title) = match view {
            PlayerView::Other {
                name,
                coins,
                revealed_cards,
            } => {
                let view = format!(
                    "Name: {}\nCoins: {}\nCards: {:?}",
                    name, coins, revealed_cards
                );

                (view, title)
            }
            PlayerView::Me { name, coins, hand } => {
                let view = format!("Name: {}\nCoins: {}\nHand: {:?}", name, coins, hand);

                (view, title.underlined())
            }
        };
        let block = Block::bordered().title_top(title);
        let paragraph = Paragraph::new(view).block(block);
        f.render_widget(paragraph, *area);
    }
}

fn draw_input_view(game_id: Uuid, ui_state: &mut UiState, area: Rect, f: &mut Frame) {
    let choices = ui_state.items.as_ref();
    let kind = choices
        .map(|c| c.kind())
        .unwrap_or("Waiting for choices...");
    let block = Block::bordered().title_top(format!("Input (Game ID: {game_id}) => {kind}"));
    let items = choices.map(|c| c.choices()).unwrap_or_default();

    let list = List::new(items).block(block).highlight_symbol(">> ");

    f.render_stateful_widget(list, area, &mut ui_state.state);
}

pub fn game_view(state: &State, ui_state: &mut UiState, f: &mut Frame) {
    if let State::InGame {
        game_id,
        player_id,
        info,
    } = state
    {
        // split screen into 2/3 for player views, and 1/3 for input box
        let layout = Layout::vertical([Constraint::Ratio(2, 3), Constraint::Ratio(1, 3)]);
        let [info_area, input_area] = layout.split(f.area())[..]
            .try_into()
            .expect("Two constraints provided");

        draw_info_view(*player_id, info, info_area, f);
        draw_input_view(*game_id, ui_state, input_area, f);
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
