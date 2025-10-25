use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::format;
use std::time::Duration;

use crate::Message as DispatchMessage;
use crate::draw;
use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use overthrow_types::Action;
use overthrow_types::Block;
use overthrow_types::Blocks;
use overthrow_types::Card;
use overthrow_types::Challenge;
use overthrow_types::Reaction;
use overthrow_types::{ClientMessage, ClientResponse, Info, PlayerId, Summary};
use ratatui::Frame;
use ratatui::text::Text;
use ratatui::widgets::List;
use ratatui::widgets::ListState;
use tokio::time::interval;
use tokio::{select, sync::mpsc};
use uuid::Uuid;

#[derive(Debug)]
struct Context<'a> {
    pub sender: &'a mut mpsc::Sender<Message>,
    pub player_id: &'a mut Option<PlayerId>,
    pub state: &'a mut State,
    pub ui_state: &'a mut UiState,
}

#[derive(Debug)]
pub enum Message {
    Response(ClientResponse),
    Quit,
}

#[derive(Debug, Default)]
pub struct UiState {
    pub items: Option<Choices>,
    pub state: ListState,
}

impl UiState {
    pub fn set(&mut self, items: Choices) {
        self.items = Some(items);
        self.state = ListState::default();
    }

    pub fn reset(&mut self) {
        self.items = None;
        self.state = ListState::default();
    }
}

#[derive(Debug)]
pub enum Choices {
    Actions(Vec<Action>),
    Reactions(Vec<Reaction>),
    Blocks(Blocks),
    Challenge(Challenge),
    Victim([Card; 2]),
    OneFromThree([Card; 3]),
    TwoFromFour([Card; 4]),
}

impl Choices {
    pub fn kind(&self) -> &'static str {
        match self {
            Choices::Actions(..) => "Choose action",
            Choices::Reactions(..) => "Choose reaction",
            Choices::Blocks(..) => "Choose block",
            Choices::Challenge(..) => "Choose challenge",
            Choices::Victim(..) => "Choose victim card",
            Choices::OneFromThree(..) => "Exchange card",
            Choices::TwoFromFour(..) => "Exchange cards",
        }
    }

    // select item from list in UI
    pub fn select(&self, index: usize, ctx: Context) {
        let response = match self {
            Choices::Actions(actions) => {
                let action = actions[index].clone();
                Some(ClientResponse::Act(action))
            }
            Choices::Reactions(reactions) => {
                reactions.get(index).cloned().map(ClientResponse::React)
            }
            Choices::Blocks(blocks) => match blocks {
                Blocks::Other(block) => (index == 0)
                    .then_some(block)
                    .map(|b| b.claim())
                    .map(ClientResponse::Block),
                Blocks::Steal(b1, b2) => (index == 0)
                    .then_some(b1)
                    .or((index == 1).then_some(b2))
                    .map(|b| b.claim())
                    .map(ClientResponse::Block),
            },
            Choices::Challenge(..) => (index == 0).then_some(ClientResponse::Challenge),
            Choices::Victim(cards) => cards.get(index).copied().map(ClientResponse::ChooseVictim),
            Choices::OneFromThree(cards) => {
                cards.get(index).copied().map(ClientResponse::ExchangeOne)
            }
            Choices::TwoFromFour(..) => todo!(),
        };

        let message = Message::Response(response.unwrap_or(ClientResponse::Pass));

        ctx.sender
            .try_send(message)
            .expect("Should always have capacity");

        ctx.ui_state.reset();
    }

    fn block(block: &Block) -> Text<'static> {
        let actor = block.actor();
        let claim = block.claim();
        let kind = block.kind();
        Text::raw(format!("As {claim}, Block Player {actor}'s Action: {kind}"))
    }

    fn challenge(challenge: &Challenge) -> Text<'static> {
        let actor = challenge.actor();
        let kind = challenge.kind();
        Text::raw(format!("Challenge Player {actor}'s Action: {kind}"))
    }

    pub fn choices(&self) -> Vec<Text<'_>> {
        use std::iter;
        match self {
            Choices::Actions(actions) => actions
                .iter()
                .map(|action| {
                    let kind = action.kind();
                    let claim = action
                        .claim()
                        .map(|c| format!("{:?}", c))
                        .unwrap_or("Player".into());

                    Text::raw(format!("As {claim}: {kind}"))
                })
                .collect(),
            Choices::Reactions(reactions) => reactions
                .iter()
                .map(|reaction| match reaction {
                    Reaction::Challenge(challenge) => Choices::challenge(challenge),
                    Reaction::Block(block) => Choices::block(block),
                })
                .chain(iter::once(Text::raw("Pass")))
                .collect(),
            Choices::Blocks(blocks) => match blocks {
                Blocks::Other(block) => vec![Choices::block(block), Text::raw("Pass")],
                Blocks::Steal(b1, b2) => {
                    vec![Choices::block(b1), Choices::block(b2), Text::raw("Pass")]
                }
            },
            Choices::Challenge(challenge) => {
                vec![Choices::challenge(challenge), Text::raw("Pass")]
            }
            Choices::Victim(cards) => cards.map(|c| Text::raw(format!("Card: {c}"))).into(),
            Choices::OneFromThree(cards) => cards.map(|c| Text::raw(format!("Card: {c}"))).into(),
            Choices::TwoFromFour(_) => todo!(),
        }
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

pub async fn ui(mut sender: mpsc::Sender<Message>, mut receiver: mpsc::Receiver<DispatchMessage>) {
    let mut term = ratatui::init();

    // 60 frames per second
    let render_delay = Duration::from_secs_f64(1.0 / 60.0);
    let mut interval = interval(render_delay);
    let mut player_id = None;
    let mut state = State::Connecting;
    let mut ui_state = UiState::default();

    term.clear().expect("Should be able to clear");

    loop {
        select! {
            msg = receiver.recv() => {
                let Some(msg) = msg else { break };
                let context = Context {
                    sender: &mut sender,
                    player_id: &mut player_id,
                    state: &mut state,
                    ui_state: &mut ui_state,
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
                term.draw(|f| draw(&state, &mut ui_state, f)).expect("Drawing should not fail");
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
        Msg::ChallengeChoice(challenge) => ctx.ui_state.set(Choices::Challenge(challenge)),
        Msg::BlockChoices(blocks) => ctx.ui_state.set(Choices::Blocks(blocks)),
        Msg::ReactionChoices(reactions) => ctx.ui_state.set(Choices::Reactions(reactions)),
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
                items.select(index, ctx);
            }
        }
        _ => {}
    }
    GamePhase::Continue
}
