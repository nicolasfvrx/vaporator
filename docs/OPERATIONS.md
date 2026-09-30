# Operations guide

## Discord setup

Create an application and bot in the [Discord Developer Portal](https://discord.com/developers/applications). Install it on your server with the `bot` and `applications.commands` OAuth2 scopes. In Discord, enable Developer Mode and copy your server ID into `DISCORD_GUILD_ID`.

The bot needs View Channel, Send Messages, Embed Links, and Attach Files in each destination channel. The optional mention role must be mentionable, unless the bot has Mention Everyone in that channel. Vaporator never permits arbitrary user or everyone mentions in generated notifications.

Commands are registered only in the configured server and checked for administrator permission at runtime. No Message Content or Server Members privileged intent is required. The database is bound to the first configured server ID; use a separate volume for a different server.

## Configuration

Copy `.env.example` to `.env` and replace the example credentials. The file is excluded from Git and the Docker build context.

| Setting | Default | Meaning |
| --- | --- | --- |
| `DISCORD_TOKEN` | Required | Discord bot token |
| `DISCORD_GUILD_ID` | Required | Discord server ID |
| `DATA_DIR` | `data` locally; `/data` in Docker | SQLite and Steam session directory |
| `BUILD_INTERVAL_SECONDS` | `60` | PICS polling interval; minimum 30 |
| `NEWS_INTERVAL_SECONDS` | `300` | Official news polling interval; minimum 60 |
| `RUST_LOG` | `vaporator=info,warn` | Logging filter |

Run one instance per database. The Docker image runs as UID 10001 and uses a persistent named volume. No inbound ports are needed. Outbound HTTPS and Steam connectivity are required.

## Commands

All replies to configuration commands are private to the administrator. Notification destinations must be text or announcement channels, not threads or forums.

| Command | Options and behavior |
| --- | --- |
| `/steam follow` | Required `app_id`, `channel`; optional `mode` (`builds`, `news`, `both`), `branch` (`public`), `role`, `news_app_id` (tracked AppID) |
| `/steam edit` | Required subscription `id`; optional `channel`, `mode`, `branch`, `role`, `clear_role`, `news_app_id` |
| `/steam remove` | Required `id`; deletes the subscription and cancels its pending deliveries |
| `/steam list` | Lists subscription IDs and routing |
| `/steam branches` | Required `app_id`; lists accessible branches and builds |
| `/steam dayz` | Required `channel`, optional `role`; adds missing DayZ subscriptions without changing existing ones |
| `/steam status` | Shows Steam connection, latest checks, queued deliveries, and monitoring errors |
| `/steam test` | Required `channel`; sends a sample without role mentions |

Adding a subscription establishes a silent baseline. Editing deliberately resets that baseline, cancels pending notifications for that subscription, and suppresses historical news. An in-flight send may complete before an edit or removal acquires the delivery lock.

`/steam dayz` follows public builds of `221100`, `1024020`, `223350`, and `1042420`. Client subscriptions also use the main DayZ news feed (`221100`). Server subscriptions track builds only. If a build is inaccessible, that preset subscription remains pending and the first successful observation becomes its baseline. If preset setup is interrupted, rerun it to add the remaining subscriptions.

Build notifications compare build IDs rather than all metadata. A rollback is a notification-worthy change. The bot performs a full reconciliation after connecting and every 15 minutes, and retries inaccessible subscriptions. Steam changes and news are independent; the bot does not infer that a news post describes a particular build.

News polling uses the official `steam_community_announcements` feed and fetches the past 24 hours with pagination. The same article is posted once per channel, even when several subscriptions share a feed. Steam's `is_external_url` flag does not determine whether an article is official. Source article text keeps its original language.

## Notification appearance

Announcements are normal Discord messages, not embeds. Steam HTML and BBCode are converted to readable headings, paragraphs, emphasis, lists, and links. The title, publication time, and source link remain visible when long article text is shortened to fit Discord's 2,000-character limit. Automatic link previews are suppressed.

Up to four unique Steam-hosted article images are uploaded as attachments below the text. JPEG, PNG, GIF, and WebP are supported, with a 2 MiB limit per image. Unavailable, oversized, or unsupported images are skipped so the text can still be delivered. Images hosted outside the supported Steam domains remain available in the original article.

Build notifications use a green embed with the game name, the Steam icon as a right-hand thumbnail when available, the branch and detection time, and separate previous/new build fields. The title links to the application's Steam Community page.

Existing databases upgrade automatically. Previously queued notifications remain readable; older queued build events may have no icon. Icons for existing subscriptions are populated on subsequent Steam metadata refreshes.

## Steam authentication

Anonymous access is the default. Test what Steam exposes:

```sh
docker compose run --rm vaporator probe
docker compose run --rm vaporator news-probe --days 30
```

For applications requiring account access, stop the bot, authenticate interactively, and restart:

```sh
docker compose stop vaporator
docker compose run --rm vaporator steam-login
docker compose run --rm vaporator probe
docker compose up -d
```

Enter your Steam account name and password at the hidden terminal prompts, then complete Steam Guard using the supported code or device confirmation flow. Never enter credentials in Discord or in command arguments. Passwords are not saved. The reusable session is stored at `/data/steam-session.json` with restricted Linux permissions.

A saved session is used on subsequent connections. If it expires or is rejected, `/steam status` reports the connection failure; rerun `steam-login`. The bot does not silently fall back to anonymous access and hide a lost account entitlement. Authentication alone cannot grant an account access to an application it does not own or otherwise have rights to.

For native runs, use `cargo run -- steam-login` and `cargo run -- probe`. Protect the data directory with your operating system's access controls, especially on Windows. To return to anonymous mode, stop the bot and move `steam-session.json` to a secure location outside the data directory.

## Deployment and updates

```sh
docker compose up -d --build
docker compose logs --tail 100
```

Docker restarts the process after a failure. Steam reconnections use progressive backoff up to five minutes. Failed Discord notifications remain queued with progressive retry delays up to one hour. `/steam status` reports channels with failed pending deliveries even after a restart. Correct the permissions and allow the next retry to run.

Build IDs and outgoing events are saved transactionally. Delivery acknowledgments are persisted separately after Discord accepts the message, so an interrupted acknowledgment can cause a duplicate. Do not run multiple bot processes against the same database.

## Backups

The volume contains `vaporator.sqlite3`, SQLite WAL files when active, and the optional Steam session. Treat backups containing a session as credentials.

For a consistent backup, stop the service first and copy the entire data directory from its stopped container:

```sh
docker compose stop vaporator
mkdir -p backups
docker compose cp vaporator:/data ./backups/vaporator-data
docker compose up -d
```

Choose a fresh destination for each backup. To restore, stop the service, copy the saved contents into its data directory, preserve UID 10001 ownership and restrictive permissions, then start the service. Never use `docker compose down -v` unless you intend to delete all persistent data.

## Troubleshooting

- **Commands are missing:** verify the configured server ID, installation scopes, administrator permissions, and command-registration log message.
- **Unknown application or no branch:** inspect `/steam branches` or `probe`. Check Steam account access. Password-protected branches are outside V1.
- **Experimental news returns 403:** use `news_app_id:221100` for DayZ; Steam account login does not authenticate the public news API.
- **No historical announcements:** this is expected at initial setup and after edits. Build monitoring announces subsequent observed changes; news recovery covers the past 24 hours.
- **A source is temporarily unavailable:** other subscriptions continue polling. Inspect `/steam status` and logs, then retry after the source recovers.
- **News pagination fails:** the API provides a timestamp cursor. A page saturated with 100 articles in the same second cannot be safely advanced; the poll fails without saving partial results and retries later.
- **Notifications do not arrive:** use `/steam test`, check channel permissions, and check the queued count and failed channels in `/steam status`.

## Internationalization

V1 ships English only. Bot-authored text lives in `locales/en.json` and uses stable keys and named placeholders. The renderer defaults and falls back to English. Discord command identifiers remain English. Future language selection must route through the renderer; no automatic article translation is performed.

Discord displays native timestamps in each reader's timezone. Logs and stored timestamps use UTC. Application names and source text preserve Unicode.
