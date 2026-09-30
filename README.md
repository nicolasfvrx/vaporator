# Vaporator

A Discord bot written in Rust to track Steam application builds and official news.

## Features

- Track Steam build changes through a persistent PICS connection, including rollbacks.
- Follow official Steam Community announcements, independently of build notifications.
- Configure applications, branches, destination channels, and optional role mentions through administrator slash commands.
- Use one bot across multiple Discord servers, with subscriptions managed separately in each server.
- Install a DayZ preset covering the stable and experimental clients and servers.
- Persist subscriptions, baselines, and pending deliveries in SQLite. Retry failures after restarts.
- Run on Linux with Docker, or directly with Rust 1.88 or newer.

## Quick start

1. Create a bot in the [Discord Developer Portal](https://discord.com/developers/applications), obtain its bot token, and install it on your server with the `bot` and `applications.commands` scopes.
2. Grant the bot **View Channel**, **Send Messages**, **Embed Links**, and **Attach Files** in the destination channels. No privileged gateway intents are required.
3. Copy `.env.example` to `.env`, then set `DISCORD_TOKEN`. Keep the token private. No server ID is required; slash commands are registered globally.
4. Start the bot:

```sh
docker compose up -d --build
docker compose logs -f
```

5. As a server administrator, run `/steam status`, then `/steam dayz channel:#steam-updates` or `/steam follow app_id:221100 channel:#steam-updates`.
6. Run `/steam test channel:#steam-updates` to verify delivery without mentioning anyone.

See the [operations guide](docs/OPERATIONS.md) for Steam login, command options, backups, and troubleshooting. The [V1 plan](docs/PLAN.md) records the intended behavior.

### DayZ access

Live anonymous testing retrieved public builds for DayZ (`221100`), DayZ Server (`223350`), and DayZ Experimental Server (`1042420`). DayZ Experimental (`1024020`) returned no accessible branches. Use `steam-login` with a Steam account that has the necessary access, then verify with `probe`.

The DayZ preset keeps inaccessible build subscriptions pending and reports them in `/steam status`. It uses the main DayZ announcement feed for both clients because the Experimental news API returned HTTP 403 during validation. Shared articles are posted once per channel.

## Local development

```sh
cargo run -- probe
cargo run -- news-probe --days 30
cargo run -- run
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

On Linux, native dependencies require a C/C++ toolchain and CMake (for example `build-essential cmake clang pkg-config`). Docker installs its build dependencies automatically. Windows builds require the MSVC build tools and CMake.

## Development

- `master` will hold the working V1 release.
- `dev` is the development branch. Commit and push each coherent, verified change to `origin/dev`.
- Merge `dev` into `master` once V1 is functional and validated.
- Use English for repository content, documentation, code comments, commit messages, and bot commands and messages. Steam articles retain their original language.

## Limits

Announcements use a heading message with the first image, followed by an article preview with the source link and remaining images when the article has text. Up to four Steam-hosted images are attached in total. Long previews are shortened to fit Discord's 2,000-character message limit. Build updates use a green embed with the game icon on the right and separate previous/new build fields.

V1 runs one bot process and database for multiple Discord servers. Administrators manage only their server's subscriptions; `/steam status` reports shared process health, pending deliveries, and errors across all servers. It does not monitor Workshop items, access password-protected branches, install updates, or restart game servers. PICS reports observed state; intermediate builds during an outage may be unavailable. Discord can receive duplicates if delivery is interrupted after sending a message but before recording success, including between the two messages of an article.

## Language and internationalization

V1 uses English throughout its interface, including Discord commands, options, descriptions, help, replies, errors, and notifications. Bot-authored display text uses the central message catalog in `locales/en.json`, with English as the default and fallback. Command identifiers remain in English. Original Steam articles are quoted in their source language.
