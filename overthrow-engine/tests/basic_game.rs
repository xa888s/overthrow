use overthrow_engine::action::Act;
use overthrow_engine::machine::ActionKind;
use overthrow_engine::machine::ChooseVictimCardState;
use overthrow_engine::machine::CoupGame;
use overthrow_engine::machine::GameState;
use overthrow_engine::machine::OnlyChallengeableState;
use overthrow_engine::machine::ReactableState;
use overthrow_engine::machine::SafeState;
use overthrow_engine::machine::Wait;
use overthrow_engine::machine::WaitState;
use overthrow_engine::players::PlayerId;
use overthrow_engine::players::RawPlayers;

#[test]
fn basic_round() {
    let players = RawPlayers::with_names(["Dave", "Garry"]).expect("Valid length");
    let game = CoupGame::with_players(players);

    let action = {
        let actions = game.actions();

        actions
            .basic()
            .iter()
            .find(|action| action.kind() == Act::Income)
            .expect("Always possible")
            .clone()
    };

    let ActionKind::Safe(game) = game.play(action) else {
        panic!("Should be a safe action")
    };

    let GameState::Wait(_game) = game.advance() else {
        panic!("Should return to game loop")
    };
}

// to get the shortest coup game, we play with two players. One player will make challengeable actions on every turn,
// and the other player will challenge them and be wrong everytime. This makes the game endable in two turns:
//
// let player one go first:
// Player 1 => claim duke and take tax, get challenged and lose
// Player 2 => steal from Player 1, then get challenged by Player 1 and win, causing Player 1 to lose their last card
//
// If we have no knowledge of player cards and control both players, the shortest coup game can be won through one
// player claiming both assassin and duke, with the other players actions not really mattering (as long as they don't
// take the first player's coins)
//
// Player 1 => claim duke and take tax
// Player 2 => (doesn't matter)
// Player 1 => assassinate Player 2, leaving 2 coins
// Player 2 => (doesn't matter)
// Player 1 => take income or tax, doesn't matter
// Player 2 => (doesn't matter)
// Player 1 => assassinate Player 2, ending the game
#[test]
fn basic_game() {
    let players = RawPlayers::with_names(["Dave", "Garry"]).expect("Valid length");
    let game = CoupGame::with_players(players);
    let take_basic_act = |game: &CoupGame<Wait>, act: Act| {
        let actions = game.actions();

        actions
            .basic()
            .iter()
            .find(|action| action.kind() == act)
            .expect("Always possible if you lie ;)")
            .clone()
    };

    let take_assasinate_act = |game: &CoupGame<Wait>, act: Act| {
        let actions = game.actions();

        actions
            .assassinations()
            .iter()
            .find(|action| action.kind() == act)
            .expect("Always possible if you lie ;)")
            .clone()
    };

    let info = game.info();
    let first_player = info.current_player;
    let victim = if first_player == PlayerId::One {
        PlayerId::Two
    } else {
        PlayerId::One
    };

    // Player 1
    let action = take_basic_act(&game, Act::Tax);

    let ActionKind::OnlyChallengeable(game) = game.play(action) else {
        panic!("Should be a challengeable action")
    };

    let GameState::Wait(game) = game.advance() else {
        panic!("Should return to game loop")
    };

    // Player 2
    let action = take_basic_act(&game, Act::Income);

    let ActionKind::Safe(game) = game.play(action) else {
        panic!("Should be a safe action")
    };

    let GameState::Wait(game) = game.advance() else {
        panic!("Should return to game loop")
    };

    // Player 1
    let action = take_basic_act(&game, Act::Tax);

    let ActionKind::OnlyChallengeable(game) = game.play(action) else {
        panic!("Should be a challengeable action")
    };

    let GameState::Wait(game) = game.advance() else {
        panic!("Should return to game loop")
    };

    // Player 2
    let action = take_basic_act(&game, Act::Income);

    let ActionKind::Safe(game) = game.play(action) else {
        panic!("Should be a safe action")
    };

    let GameState::Wait(game) = game.advance() else {
        panic!("Should return to game loop")
    };

    // Player 1
    let action = take_assasinate_act(&game, Act::Assassinate { victim });

    let ActionKind::Reactable(game) = game.play(action) else {
        panic!("Should be a reactable action")
    };

    let GameState::ChooseVictimCard(game) = game.advance() else {
        panic!("Should ask for which card to choose from victim")
    };

    let [c1, _] = game.choices();
    let game = game.advance(c1);

    // Player 2
    let action = take_basic_act(&game, Act::Income);

    let ActionKind::Safe(game) = game.play(action) else {
        panic!("Should be a safe action")
    };

    let GameState::Wait(game) = game.advance() else {
        panic!("Should return to game loop")
    };

    // Player 1
    let action = take_assasinate_act(&game, Act::Assassinate { victim });

    let ActionKind::Reactable(game) = game.play(action) else {
        panic!("Should be a reactable action")
    };

    let GameState::End(_) = game.advance() else {
        panic!("Should finish game")
    };
}
