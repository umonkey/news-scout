# Configuration

## Reader bot

To read messages from your subscription, you need to create a new Telegram App at `my.telegram.org`, and pass its id and hash via the environment variables:

```bash
API_ID='foo'
API_HASH='bar'
```

The reader watches only the chats that belong to a Telegram folder, and skips one-to-one private chats. Set the folder name with `NEWS_FOLDER`, which defaults to `News`:

```bash
NEWS_FOLDER='News'
```

Only chats in that folder are stored, queued, and marked as read. Folder membership is resolved from the folder's pinned and included peers, minus its excluded peers, and is refreshed without restarting the process. If the folder does not exist, the reader fails closed and processes nothing. Existing `EXCLUDE_*` rules still apply within the folder.

On startup the reader also recovers messages that arrived while it was offline. Telegram's built-in catch-up only covers a bounded window, so for each monitored chat the reader additionally backfills history newer than the newest message already stored locally. Backfilled messages go through the same store-and-queue path as live ones, are never marked as read, and are skipped entirely for a chat that has no stored message yet (first run), to avoid posting its whole history.

## Sender bot

Messages, such as reports, are sent via a separate bot account. You need to create one via [@BotFather](https://t.me/BotFather), then set the bot hash and your chat id with the environment variables. BotFather will tell you the bot hash, but to get the chat id, you need to do the following:

1. Send the `/start` message to your new bot, then a random text message.
2. Use curl to get `https://api.telegram.org/bot<TOKEN>/getUpdates`. In the response, look for the `id` in the `chat` block. That's the id of your personal with the bot.

Pass them to the bot via the environment variables:

```bash
REPORT_CHAT_ID='12345'
REPORT_TOKEN='foo:bar'
```

`REPORT_CHAT_ID` accepts either a plain chat id, as above, or a `chat_id:thread_id` pair for chats with forum topics enabled. The thread id is the message thread id of the topic where reports should be delivered. For example:

```bash
REPORT_CHAT_ID='-12345:1'
```

## LLM send step

Messages that pass the keyword filter are not posted directly. The reader queues them in the local `news_candidates` table, and the `send` command runs as a daemon that processes the queue one candidate at a time. For each candidate it makes a single LLM call, and:

- if the model replies with `[IGNORE]`, the candidate is dropped;
- otherwise the model output is posted to the report target, followed by a blank line and the source link.

LLM or Telegram failures are reported to Sentry, the candidate is kept, and the daemon retries after a delay.

Every command resolves the system prompt once at startup: it is read from `analyze-prompt.md` in the working directory, unless overridden by `NEWS_SCOUT_PROMPT` (inline text) or `NEWS_SCOUT_PROMPT_FILE` (a path whose contents are read). A missing prompt (no override and no default file) is a hard error. Tune it to control what the model ignores and how it rewrites the approved messages.

It requires `OPENROUTER_API_KEY` and uses `OPENROUTER_MODEL` (defaults to `google/gemini-3.8-flash`).

## Exclusions

Within the configured folder, the reader stores every group and channel message it receives, queues messages that match a keyword, and marks them as read. The account's own outgoing messages are never queued. One optional environment variable accepts a comma-separated list of sender ids whose messages are ignored completely: they are not stored, queued, or marked as read. User and bot ids are positive, while chat ids are negative.

- `EXCLUDE_FROM`: senders whose messages are ignored completely.

For example:

```bash
EXCLUDE_FROM='123456789,987654321'
```
