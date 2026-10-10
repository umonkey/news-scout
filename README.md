# News Scout

This repository contains a Telegram client which monitors all chats from folder "News" (configurable), detects messages related to trees or other urban greenery, and forwards them to the headquarters (configurable). No activity is ever made on behalf of the main account: notifications are sent using a separate Telegram Bot API account.

## Tech Stack

The app is written in Rust. The internal database is in SQLite. It uses OpenRouter to post-process messages after basic keyword filtering.

## Implemented functions

- Reads and indexes messages from the chats in the configured Telegram folder.
- Queues messages with certain keywords, except the account's own outgoing messages, for the LLM step.
- Runs the queued candidates through an LLM prompt and posts the approved messages to the headquarters chat, dropping the ones the model marks as `[IGNORE]`.
- Marks messages from groups and channels as read.
- On startup, catches up on messages that arrived while the reader was offline: it lets Telegram's built-in catch-up deliver missed updates, then backfills each monitored chat's history newer than the last stored message.
- Skips messages from excluded senders entirely.
- Removes messages from the local database when they are deleted in Telegram.
- Report any errors to a Sentry account.

## Commands

The application takes a single command as its first argument:

- `run`: the mode the container uses. Starts the reader and the sender together in one process, each supervised in place so a crash restarts it with backoff without disturbing the other. A fatal error stops both and exits non-zero, letting the container restart policy take over.
- `read`: start only the reader, which watches Telegram and queues matching messages.
- `send`: start only the send daemon, which processes queued candidates with the LLM system prompt from `analyze-prompt.md` (overridable) and posts the approved messages.
- `send-test-report`: send a test message to the configured report target.

## Setting up

1. Sign up for an application at [my.telegram.org](https://my.telegram.org/apps). Specify "other" as the type. Record app id and hash.
2. Put them in the environment variables `API_ID` and `API_HASH`.
3. Configure other environment variables listed below.
4. Run the app, it will guide you through the log in phase using the QR-code login flow.

## Runtime Configuration

- `API_HASH`: a long hash from my.telegram.org.
- `API_ID`: a numeric app identifier from my.telegram.org.
- `DATABASE_PATH`: the name of the SQLite database to store the chat messages. Defaults to `var/history.db`.
- `EXCLUDE_FROM`: comma-separated sender ids whose messages are ignored completely.
- `MAX_MESSAGE_AGE`: how long to keep the messages for, in seconds. Defaults to 7 days.
- `NEWS_FOLDER`: the name of the Telegram folder whose chats are watched. Defaults to `News`. Only chats in this folder are stored, queued, and marked read.
- `NEWS_SCOUT_PROMPT`: the system prompt used by the `send` command, given inline.
- `NEWS_SCOUT_PROMPT_FILE`: a file whose contents are used as the `send` system prompt, read once at startup.
- `OPENROUTER_API_KEY`: the OpenRouter API key used by the `send` command.
- `OPENROUTER_MODEL`: the OpenRouter model slug used by the `send` command. Defaults to `google/gemini-3.7-flash`.
- `REPORT_CHAT_ID`: the chat where the bot sends reports. Either a plain chat id (`-12345`) or a `chat_id:thread_id` pair (`-12345:1`) to post to a forum topic.
- `REPORT_TOKEN`: the bot API token for responses.
- `SENTRY_DSN`: the URL to send [Sentry](https://sentry.io/) errors to.
- `SESSION_FILE`: a file used by the Telegram connector to store the authentication state. Keep it on a secure persistent storage. Defaults to `var/session.dat`.

Every option can be configured via a file instead of a direct environment variable using the `_FILE` suffix. For example, if `API_HASH` has to be read from a file named `/run/secrets/api_hash`, set `API_HASH_FILE=/run/secrets/api_hash`.

The system prompt is resolved once at startup by every command: it is read from `analyze-prompt.md` in the process working directory (the image copies it to `/app`) unless `NEWS_SCOUT_PROMPT` or `NEWS_SCOUT_PROMPT_FILE` overrides it. A missing prompt (no override and no default file) is a hard error.
