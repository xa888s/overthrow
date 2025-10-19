# Creating a client
## Preface
You should first read about the basic game loop in the `README` to understand how the server works. After that, you should have enough background to understand the high level requirements of a client.

## Connecting to the server
Connection to the server is currently quite simple (`TODO: add lobbies`). There is no persistent state or names, rather a client is automatically assigned to a lobby when they open a WebSocket connection to `YOUR_SERVER_URL:3000/websocket` (if testing on your local machine, this URL would be ws://localhost:3000/websocket). For example in a browser you can run some JavaScript:
```js
// open connection to WebSocket
const websocket = new WebSocket("ws://localhost:3000/websocket")

// onmessage is called whenever a message is received, with its data field containing the actual message text
websocket.onmessage = function (msg) {
    console.log("Message: " + msg.data);
}

// define your other callbacks here
// ...
```
Which will immediately try to connect to the server, and print out any messages received. Please consult the [WebSocket docs](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket) for more information on usage. 

Anyways, once a connection is succesfully established with the server, nothing will happen until enough players join (`TODO: add a mechanism like a ready up button`), which currently is just 2. After another player joins, the server will start the game and send each client their first message: their player id. Let's quickly review the message format

### Message format
Each message sent and received from the server will have the following format:
```
{ messageKind: messageData }
```
Where `messageKind` is a string, and `messageData` can be either a string, array of objects, etc. depending on the given message kind. As an example, a player id message sent to player "One" looks like this:
```json
{ "PlayerId": "One" }
```
Each time I mention a client message/response, I will give an example for clarity, and state its `messageKind` and form of `messageData`

## First action
After a game has started, the `PlayerId` message is sent to all players, containing their respective IDs. The current player (who is selected at random), will then be sent a `ActionChoices` message immediately after. The format is as follows:
```
{ "ActionChoices": [Action, Action, ..] }
```
Where an action is:
```
{ "actor": PlayerId, "kind": Act }
```
All of these types are detailed and specified in the `client_message.json` and `client_response.json`, which are in the JSONSchema format that can be used to generate type definitions automatically (see: `README`). They are relatively straightforward to read if you want to know more. You can also consult the `client.rs` file. Specifically, look at the `ClientMessage` and `ClientResponse` enums, which detail all possible messages and responses.

After receiving the message, the current player can respond with an `Act` message:
```
// for example, if we are provided with these actions:
{ "ActionChoices": [{ "actor": "One", "kind": "Income" }, ..] }

// we can choose the income action by sending this message:
{ "Act": { "actor": "One", "kind": "Income" } }
```
Once an action has been received from the current player, we move on to the reaction phase.

## Reactions 
Note that just choosing an action doesn't automatically go through, instead we enter a "reaction" phase, where the other players are sent their possible reactions to a given action. In this phase, they have 10 seconds to send a reaction before the action automatically passes. This corresponds to the `ReactionChoices`, `ChallengeChoice`, and `BlockChoices` `messageKind`s, which has a list of possible reactions. The client can then respond with a `React`, `Challenge`, or `Block` response, choosing one of those reactions.

### Dual phase
Importantly, certain reactions can be reacted to (i.e. a player `A` chooses to block player `B`'s steal action by claiming they are an ambassador, which player `B` can then challenge). These re-reactions will be immediately sent after a reaction is chosen.

## Victims
Some actions'/reactions' effects include an exchange of cards (or losing them). In these cases, the victim or actor will choose from a selection of cards to keep. These correspond to the `VictimChoices`, `OneFromThreeChoices`, and `TwoFromFourChoices`. The client then responds with `ChooseVictim`, `ExchangeOne`, and `ExchangeTwo` responses respectively.

## End of round
At the end of each round, after all actions, reactions, and choices have gone through, an `Outcome` message is sent detailing what happened, and an `Info` message is sent containing views of the other players, who the current player is (for the next round), and the coins remaining in the pile.

## Cancelled
If any player leaves the game, the game is cancelled and a `GameCancelled` message is sent to all remaining players.