use std::time::Duration;

use crate::Message as DispatchMessage;
use crate::draw;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use overthrow_types::{ClientMessage, ClientResponse, Info, PlayerId, Summary};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use tokio::time::interval;
use tokio::{select, sync::mpsc};
use uuid::Uuid;

#[derive(Debug)]
struct Context<'a> {
    pub sender: &'a mut mpsc::Sender<Message>,
    pub receiver: &'a mut mpsc::Receiver<DispatchMessage>,
    pub player_id: &'a mut Option<PlayerId>,
    pub state: &'a mut State,
}

#[derive(Debug)]
pub enum Message {
    Response(ClientResponse),
    Quit,
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

pub async fn ui(mut sender: mpsc::Sender<Message>, mut receiver: mpsc::Receiver<DispatchMessage>) {
    let mut term = ratatui::init();

    // 60 frames per second
    let render_delay = Duration::from_secs_f64(1.0 / 2.0);
    let mut interval = interval(render_delay);
    let mut player_id = None;
    let mut state = State::Connecting;
    term.clear().expect("Should be able to clear");

    loop {
        select! {
            msg = receiver.recv() => {
                let Some(msg) = msg else { break };
                let context = Context {
                    sender: &mut sender,
                    receiver: &mut receiver,
                    player_id: &mut player_id,
                    state: &mut state,
                };
                let state = match msg {
                    DispatchMessage::Server(msg) => handle_server_event(msg, context),
                    DispatchMessage::Term(event) => handle_term_event(event, context),
                    DispatchMessage::Quit => break,
                };

                // we have reached the end
                if let GamePhase::End(_) | GamePhase::Cancelled = state {
                    break;
                }
            },
            _ = interval.tick() => {
                term.draw(|f| draw(&state, f)).expect("Drawing should not fail");
            },
            else => break,
        }
    }

    sender
        .send(Message::Quit)
        .await
        .expect("Should be able to send quit message");

    ratatui::restore();
}

fn draw(state: &State, f: &mut Frame) {
    draw::splash_page(state, f);
    draw::game_view(state, f);
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
        Msg::Outcome(outcome) => todo!(),
        Msg::ActionChoices(actions) => eprintln!("Choosing actions..."),
        Msg::ChallengeChoice(challenge) => todo!(),
        Msg::BlockChoices(blocks) => todo!(),
        Msg::ReactionChoices(reactions) => todo!(),
        Msg::VictimChoices(_) => todo!(),
        Msg::OneFromThreeChoices(_) => todo!(),
        Msg::TwoFromFourChoices(_) => todo!(),
    }

    GamePhase::Continue
}

fn handle_term_event(event: Event, ctx: Context) -> GamePhase {
    use Event as E;
    match event {
        E::Key(event) if event.code == KeyCode::Char('q') => return GamePhase::Cancelled,
        E::Key(_) => todo!(),
        // ignore
        E::FocusGained | E::FocusLost | E::Paste(..) | E::Mouse(..) | E::Resize(..) => {}
    }

    GamePhase::Continue
}
