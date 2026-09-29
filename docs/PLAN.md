# V1 Plan — Vaporator

## Goal

Build a Discord bot in Rust for one community, configurable through administrator slash commands. Announce new Steam builds and official developer posts. Run the bot on Linux with Docker and persist state in SQLite on a mounted volume.

Use English for repository content, documentation, code comments, commit messages, and bot commands and messages. Steam articles retain their original language.

## Architecture

- Use Tokio for asynchronous execution, Poise/Serenity for Discord, steam-vent for Steam, reqwest for news, and SQLx for SQLite.
- Separate Steam collection, event detection, storage, and Discord publishing.
- First, validate PICS requests and branch/build retrieval for the four DayZ applications. Verify names, anonymous access, and any authentication requirements.
- Try anonymous Steam authentication by default. Provide `vaporator steam-login` for local authentication with Steam Guard and a protected persistent session in the Docker volume. Never send Steam credentials through Discord.

## Language and internationalization

- Ship English only in V1. Use English for command and subcommand names, option names, choice values, descriptions, help, replies, validation errors, embeds, buttons, notifications, and CLI output.
- Keep command identifiers and machine-readable values in English even when additional display languages are introduced.
- Store bot-authored display text in a central English message catalog, accessed through stable keys and named placeholders. Keep message formatting separate from business logic and avoid sentence concatenation.
- Provide a locale-aware rendering boundary with English as the default and fallback for unsupported locales or missing translations. V1 always renders English; additional translations and language selection are future work.
- Use Discord timestamps in Discord messages and UTC timestamps in logs and persisted state. Preserve Unicode in application names, user-supplied labels, and source articles.
- Keep logs, developer diagnostics, documentation, code comments, and commit messages in English. Steam articles retain their source language; automatic article translation is outside V1.

## Build monitoring

- Maintain a persistent Steam connection and query PICS changes every 60 seconds using the last stored change number.
- Fetch updated information for affected tracked applications and announce changes to the tracked branch's `buildid`, including rollbacks to an older build.
- Ignore metadata changes without a build change.
- When adding a subscription, record the current state without announcing historical updates.
- After an interruption, compare against the last stored state. Do not promise to reconstruct intermediate builds.
- Refresh all tracked applications after reconnecting and periodically to reconcile state.

## Discord commands

Restrict commands to administrators of the configured Discord server:

| Command | Purpose |
| --- | --- |
| `/steam follow` | AppID, builds/news/both, branch, channel, and optional role |
| `/steam edit` | Edit a subscription, including its news source |
| `/steam remove` | Remove a subscription |
| `/steam list` | List subscriptions and their identifiers |
| `/steam branches` | List an application's accessible branches |
| `/steam dayz` | Install the DayZ preset |
| `/steam status` | Show connections, latest checks, and errors |
| `/steam test` | Send a sample without mentions to the selected channel |

The DayZ preset tracks the `public` branch of these four applications:

| Application | AppID |
| --- | --- |
| DayZ | 221100 |
| DayZ Experimental | 1024020 |
| DayZ Server | 223350 |
| DayZ Experimental Server | 1042420 |

A build announcement includes the application, branch, previous and new build IDs, and detection time. Messages are in English. Mentions are disabled by default and limited to the explicitly configured role.

## News

- Query `ISteamNews/GetNewsForApp` every 5 minutes and filter for official Steam Community posts.
- Publish the title, a short sanitized excerpt, and the original link in the article's language.
- Allow a separate source application for a dedicated server's news.
- The DayZ preset uses the main DayZ news feed (`221100`) for both client subscriptions, deduplicated per channel. Live validation found the Experimental news endpoint (`1024020`) returns HTTP 403. Administrators can override the source with `/steam edit`.
- Publish build announcements and articles separately without assuming an automatic association.
- When adding a subscription, record existing articles without posting them. After an interruption, catch up on new articles from the past 24 hours using pagination.
- Deduplicate articles by Steam article identifier and channel.

## Persistence and errors

- Record detected changes and pending notifications in the same SQLite transaction.
- Retry failed deliveries, respect Discord rate limits, and reconnect to Steam with progressive backoff.
- Report permission issues and expired Steam sessions in logs and `/steam status`.
- A duplicate remains possible if Discord accepts a message just before an interruption prevents recording success. Do not promise exactly-once delivery.

## Delivery and validation

- Deliver the Rust project, Dockerfile, Docker Compose configuration, example configuration, and a guide covering bot setup, permissions, Steam Guard, and volume backups.
- Test build changes, rollbacks, metadata-only changes, silent initial setup, restarts, and pending notification recovery.
- Test news filtering, duplicates, pagination, and oversized content.
- Test unknown applications, inaccessible branches, insufficient Discord permissions, Steam/Discord outages, and session expiry.
- Validate that all message keys exist in the English catalog, named placeholders match, unsupported locales fall back to English, and Discord command identifiers remain English. Check Unicode labels and timestamp rendering.
- Run compilation, tests, Clippy, Docker startup, live reads of the four applications, and a test message in the configured channel.

## V1 limits

One Discord server, configurable applications beyond DayZ, and branches accessible without a password. Workshop monitoring and game server installation or restarts are out of scope.

## References

- [steam-vent](https://docs.rs/steam-vent/latest/steam_vent/)
- [PICS requests in SteamKit](https://github.com/SteamRE/SteamKit/blob/master/SteamKit2/SteamKit2/Steam/Handlers/SteamApps/SteamApps.cs)
- [Steam news API](https://partner.steamgames.com/doc/webapi/ISteamNews)
- [DayZ server hosting](https://community.bistudio.com/wiki/DayZ:Hosting_a_Linux_Server)
