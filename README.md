- Running against a csv file: `cargo run -- file://data/chat1.txt`
- Running against a twitch stream: `cargo run -- twitch://<twitch_username>` i.e. `cargo run -- twitch://northernlion`

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

## TODO
- Create an initialization bootstrap to guide the setup of API information.
- Highlight messages from logged-in user for better visibility.
