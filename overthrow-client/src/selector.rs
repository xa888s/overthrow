use overthrow_types::{Action, Blocks, Card, Challenge, Reaction};
use overthrow_types::{Block, ClientResponse};
use ratatui::text::Text;

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
    pub fn selection_to_response(&self, index: usize) -> ClientResponse {
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

        response.unwrap_or(ClientResponse::Pass)
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
