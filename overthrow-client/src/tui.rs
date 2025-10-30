use std::fmt::Debug;
use std::time::Duration;

use crate::draw;
use crate::selector::Choices;
use crate::server::LocalMessage;
use crossterm::event::Event;
use crossterm::event::EventStream;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use futures::StreamExt;
use overthrow_types::{ClientMessage, Info, PlayerId, Summary};
use ratatui::Frame;
use ratatui::widgets::ListState;
use tokio::time::interval;
use tokio::{select, sync::mpsc};
use uuid::Uuid;

#[derive(Debug)]
pub struct Context<'a> {
    pub sender: &'a mut mpsc::Sender<LocalMessage>,
    pub player_id: &'a mut Option<PlayerId>,
    pub state: &'a mut State,
    pub ui_state: &'a mut UiState,
}

#[derive(Debug, Default)]
pub struct UiState {
    pub items: Option<Choices>,
    pub state: ListState,
}

impl UiState {
    pub fn set(&mut self, items: Choices) {
        self.items = Some(items);
        // auto select first item
        self.state = ListState::default().with_selected(Some(0));
    }

    pub fn reset(&mut self) {
        self.items = None;
        self.state = ListState::default();
    }
}

#[derive(Debug)]
pub enum State {
    Connecting,
    InLobby {
        game_id: Uuid,
    },
    InGame {
        game_id: Uuid,
        player_id: PlayerId,
        info: Info,
    },
}

pub async fn ui(
    mut sender: mpsc::Sender<LocalMessage>,
    mut receiver: mpsc::Receiver<ClientMessage>,
) {
    // for terminal events
    let mut term_events = EventStream::new();
    let mut term = ratatui::init();

    // 60 frames per second
    let render_rate = Duration::from_secs_f64(1.0 / 60.0);
    let mut interval = interval(render_rate);

    // state
    let mut player_id = None;
    let mut state = State::Connecting;
    let mut ui_state = UiState::default();

    term.clear().expect("Should be able to clear");

    loop {
        let ctx = Context {
            sender: &mut sender,
            player_id: &mut player_id,
            state: &mut state,
            ui_state: &mut ui_state,
        };
        let state = select! {
            biased;
            Some(Ok(event)) = term_events.next() => handle_term_event(event, ctx),
            Some(msg) = receiver.recv() => handle_server_event(msg, ctx),
            _ = interval.tick() => {
                term.draw(|f| draw(&state, &mut ui_state, f)).expect("Drawing should not fail");
                continue;
            },
            else => break,
        };

        if let GamePhase::End(_) | GamePhase::Cancelled = state {
            break;
        }
    }

    sender
        .send(LocalMessage::Quit)
        .await
        .expect("Should be able to send quit message");

    ratatui::restore();
}

fn draw(state: &State, ui_state: &mut UiState, f: &mut Frame) {
    draw::splash_page(state, f);
    draw::game_view(state, ui_state, f);
}

#[derive(Debug)]
enum GamePhase {
    End(Summary),
    Cancelled,
    Continue,
}

fn update_info(info: Info, ctx: Context) {
    match ctx.state {
        State::Connecting => unreachable!("Info should never be sent before game_id"),
        State::InLobby { game_id } => {
            let game_id = *game_id;
            let player_id = ctx.player_id.expect("Should have player_id before info");

            *ctx.state = State::InGame {
                game_id,
                player_id,
                info,
            };
        }
        State::InGame { info: old_info, .. } => *old_info = info,
    }
}

fn handle_server_event(msg: ClientMessage, ctx: Context) -> GamePhase {
    use ClientMessage as Msg;
    match msg {
        Msg::PlayerId(player_id) => *ctx.player_id = Some(player_id),
        Msg::GameId(game_id) => *ctx.state = State::InLobby { game_id },
        Msg::Info(info) => update_info(info, ctx),
        Msg::End(summary) => return GamePhase::End(summary),
        Msg::GameCancelled => return GamePhase::Cancelled,
        // setting and resetting ui state
        Msg::Outcome(outcome) => ctx.ui_state.reset(),
        Msg::ActionChoices(actions) => ctx.ui_state.set(Choices::Actions(actions)),
        Msg::ChallengeChoice(challenge, timestamp) => {
            ctx.ui_state.set(Choices::Challenge(challenge))
        }
        Msg::BlockChoices(blocks, timestamp) => ctx.ui_state.set(Choices::Blocks(blocks)),
        Msg::ReactionChoices(reactions, timestamp) => {
            ctx.ui_state.set(Choices::Reactions(reactions))
        }
        Msg::VictimChoices(cards) => ctx.ui_state.set(Choices::Victim(cards)),
        Msg::OneFromThreeChoices(cards) => ctx.ui_state.set(Choices::OneFromThree(cards)),
        Msg::TwoFromFourChoices(cards) => todo!(),
    }

    GamePhase::Continue
}

fn handle_term_event(event: Event, ctx: Context) -> GamePhase {
    use Event as E;
    match event {
        E::Key(event) => return handle_key_event(event, ctx),
        // ignore
        E::FocusGained | E::FocusLost | E::Paste(..) | E::Mouse(..) | E::Resize(..) => {}
    }

    GamePhase::Continue
}

fn handle_key_event(event: KeyEvent, ctx: Context) -> GamePhase {
    match event.code {
        KeyCode::Char('q') | KeyCode::Esc => return GamePhase::Cancelled,
        KeyCode::Up => ctx.ui_state.state.scroll_up_by(1),
        KeyCode::Down => ctx.ui_state.state.scroll_down_by(1),
        // item has been selected
        KeyCode::Enter => {
            if let Some(items) = ctx.ui_state.items.take()
                && let Some(index) = ctx.ui_state.state.selected()
            {
                let response = items.selection_to_response(index);
                let message = LocalMessage::Response(response);

                ctx.sender
                    .try_send(message)
                    .expect("Should always have capacity");

                ctx.ui_state.reset();
            }
        }
        _ => {}
    }
    GamePhase::Continue
}
