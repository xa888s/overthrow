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