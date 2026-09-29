# Vaporator

A Discord bot written in Rust to track Steam application builds and official news.

## Project status

The repository is initialized. V1 implementation is pending; see the [plan](docs/PLAN.md).

## Development

- `master` will hold the working V1 release.
- `dev` is the development branch. Commit and push each coherent, verified change to `origin/dev`.
- Merge `dev` into `master` once V1 is functional and validated.
- Use English for repository content, documentation, code comments, commit messages, and bot commands and messages. Steam articles retain their original language.

## V1 target

One Discord server, slash command configuration, Steam monitoring through PICS, official news, SQLite storage, and Docker deployment on Linux.

## Language and internationalization

V1 uses English throughout its interface, including Discord commands, options, descriptions, help, replies, errors, and notifications. Bot-authored display text will use a central message catalog so additional languages can be added later, with English as the default and fallback. Command identifiers remain in English. Original Steam articles are quoted in their source language.
