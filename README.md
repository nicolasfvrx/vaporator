# Vaporator

A Discord bot written in Rust to track Steam application builds and official news.

## Features

- Track Steam build changes through a persistent PICS connection, including rollbacks.
- Follow official Steam Community announcements, independently of build notifications.
- Configure applications, branches, destination channels, and optional role mentions through administrator slash commands.
- Find games by name, AppID, or Steam link, with autocomplete backed by a persistent local catalog.
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

5. As a server administrator, run `/steam status`, then `/steam dayz channel:#steam-updates` or `/steam follow game:221100 channel:#steam-updates`. Omit `channel` to use the current text or announcement channel.
6. Run `/steam test channel:#steam-updates` to verify delivery without mentioning anyone.

See the [operations guide](docs/OPERATIONS.md) for Steam login, command options, backups, and troubleshooting. The [V1 plan](docs/PLAN.md) records the intended behavior.

### Game search

The development branch replaces the `app_id` option with `game` on `/steam follow`, `/steam branches`, and `/steam test`. Type a name and select a suggestion, enter an AppID, or paste a Steam application link. The optional `news_app_id` also accepts these inputs. Ambiguous names require selecting an AppID; fuzzy matches are suggestions only.

Set the optional `STEAM_WEB_API_KEY` in `.env` to enable the full Steam Store name catalog. A standard [Steam Web API user key](https://steamcommunity.com/dev/apikey) is sufficient and is separate from the saved Steam login session. The catalog refreshes in the background every six hours and is stored in SQLite. Without a key, the bot searches applications already learned from Steam or existing subscriptions; unknown AppIDs and Steam links can still be entered directly. This remains a standalone process with SQLite.

### Download a compiled release

Download the Linux x86_64 (`.tar.gz`) or Windows x86_64 (`.zip`) archive from [GitHub Releases](https://github.com/nicolasfvrx/vaporator/releases/latest). Each archive contains the executable, `.env.example`, and documentation. Rust and Docker are not required to run these binaries. Linux binaries target Ubuntu 22.04 or newer compatible systems with glibc 2.35 or later.

Extract the archive, copy `.env.example` to `.env`, and set `DISCORD_TOKEN`. From the extracted directory, run `./vaporator run` on Linux or `.\vaporator.exe run` in PowerShell. Configuration and data are read relative to the working directory. See [native installation](docs/OPERATIONS.md#native-installation) for verification and Steam login.

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
