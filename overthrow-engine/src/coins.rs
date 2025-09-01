use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(transparent)]
pub struct PlayerCoins(u8);

impl Default for PlayerCoins {
    fn default() -> Self {
        PlayerCoins(2)
    }
}

impl PlayerCoins {
    pub(crate) fn steal(mut self, mut thief: PlayerCoins) -> (PlayerCoins, PlayerCoins) {
        self.0 = self
            .0
            .checked_sub(2)
            .expect("Player should always have at least two coins");

        thief.0 += 2;

        (self, thief)
    }

    pub fn amount(&self) -> u8 {
        self.0
    }
}

#[derive(Debug)]
pub(crate) enum Withdrawal {
    Income = 1,
    ForeignAid = 2,
    Tax = 3,
}

#[derive(Debug)]
pub(crate) enum Deposit {
    Assassinate = 3,
    Coup = 7,
}

const STARTING_COINS: u8 = 50;

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct WithdrawalError {
    coins: PlayerCoins,
    amount_remaining: u8,
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct DepositError {
    coins: PlayerCoins,
    amount_remaining: u8,
}

#[derive(Debug)]
pub(crate) struct CoinPile {
    coins: u8,
}

impl CoinPile {
    pub(crate) fn with_count(
        player_count: u8,
    ) -> (CoinPile, impl IntoIterator<Item = PlayerCoins>) {
        let remaining = STARTING_COINS - (player_count * 2);
        (
            CoinPile { coins: remaining },
            (0..player_count).map(|_| PlayerCoins(2)),
        )
    }

    pub fn remaining(&self) -> u8 {
        self.coins
    }

    // when player dies, coins should be returned to coin pile
    pub(crate) fn return_coins(&mut self, PlayerCoins(coins): PlayerCoins) {
        self.coins += coins;
    }

    // takes a withdrawal request and a player's coins, then returns the modified coins
    pub(crate) fn withdraw(
        &mut self,
        withdrawal: Withdrawal,
        coins: PlayerCoins,
    ) -> Result<PlayerCoins, WithdrawalError> {
        let amount = withdrawal as u8;
        let PlayerCoins(player_coins) = coins;

        self.coins = self.coins.checked_sub(amount).ok_or(WithdrawalError {
            coins,
            amount_remaining: self.coins,
        })?;

        Ok(PlayerCoins(player_coins + amount))
    }

    // takes a spend request and a player's coins, then returns the modified coins
    pub(crate) fn spend(
        &mut self,
        deposit: Deposit,
        coins: PlayerCoins,
    ) -> Result<PlayerCoins, DepositError> {
        let amount = deposit as u8;
        let PlayerCoins(player_coins) = coins;

        let player_coins = player_coins.checked_sub(amount).ok_or(DepositError {
            coins,
            amount_remaining: player_coins,
        })?;

        self.coins += amount;

        Ok(PlayerCoins(player_coins))
    }
}
