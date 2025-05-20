## Twitch Config
To configure twitch:
1. Create a dev account and acquire the client id and secret.
2. Create a file located at `$HOME/.streamchat/api.toml`.
  - Create the `$HOME/.streamchat` directory if it doesn't already exist.
  - Store the client id and secret in the fields `twitch_api_client_id` and `twitch_api_secret`, respectively.
3. Add `http://localhost:7890` to your twitch app portal url callback whitelist.
  - This is to inform twitch that this is the url to send auth credentials to.
  - To keep the application completely local to the user, server-oauth is handled by a temporary HTTP server
    that is spun up locally to acquire user's access tokens.

At the end, your `app.toml` file should look like:
```
twitch_api_client_id = "<....>"
twitch_api_secret = "<....>"
```

## Debugging
### Profiling
Using the `samply` program (https://github.com/mstange/samply/), you can record a profile of the application
by building the release version and starting the application with samply:

```
# Install samply
cargo install --git https://github.com/mstange/samply.git samply

# Build release
RUSTFLAGS="-C force-frame-pointers=yes" cargo build --release

# Run app while recording profile
samply record target/release/streamchat
```

## TODO
- Create an initialization bootstrap to guide the setup of API information.
- When user scrolls up in the chat, do not append new messages. This should prevent the chat moving away when
  trying to read some previous message.
- The text input should be multiline so that long messages are still visible.
- Twitch: You can create a maximum of 3 WebSockets connections with enabled subscriptions. Reconnecting using a reconnection URL (see Reconnect message) doesn’t add to your WebSocket count.
  - Implement chat caching that will temp disconnect from inactive tabs and reconnect to them when
    they are refocused. (https://dev.twitch.tv/docs/eventsub/handling-websocket-events/)
