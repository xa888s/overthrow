# overthrow-server
An async, WebSocket-based server using the overthrow-engine.

## Generate TS types from JSON Schema
This project automatically generates a JSON Schema for the WebSocket messages under the `.json` files in this directory.
To generate type declaration files for TypeScript, you can use [json-schema-to-typescript](https://www.npmjs.com/package/json-schema-to-typescript).

### Example:
```shell
# first install the package
npm install json-schema-to-typescript --global

# then you can use it like so, which will generate the type files and place them in the './types/' directory
json2ts -i '*.json' -o types/
```

Ideally this would be automated, but I don't want to include `npm` as a core build dependency

## Rough game loop
The game loop roughly follows these steps on the server:

0. Initialize the game state (hand out 2 cards and 2 coins to each player, then choose starting player to set as current player)
1. Send current player their possible actions, and wait for response
2. Once chosen action is received, send other players their possible reactions
3. If after 10 seconds no players respond to the action, the action automatically passes. If another player reacts, then the current player is sent their possible reactions to that reaction to choose from, if they exist.
4. After all possible actions/reactions are settled, their effects are applied to the game state (e.g. losing/gaining coins, player dies, etc.), and the outcome is sent to all clients, along with the info of each player after these changes have been applied (i.e. remaining coins, revealed cards, etc.)
5. We then go to step 1 and repeat, until there is only one player left standing, where we then send an end message that details a summary of the game (who won)