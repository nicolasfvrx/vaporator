# Working instructions

## Git

- Work on `dev` and preserve the user's existing changes.
- After each coherent change, run appropriate checks, create a commit, and push to `origin/dev`.
- Do not commit secrets, Steam sessions, or local databases.
- Do not push development changes to `master`. Reserve the merge into `master` for a functional, validated V1.
- Do not rewrite published history or force push.

## Product and language

- Follow the decisions in `docs/PLAN.md`.
- Write all repository content in English, including documentation, code comments, identifiers, and commit messages.
- Use English for bot commands and messages. Steam articles retain their original language.
- Keep command names, subcommand names, option names, and machine-readable values in English.
- Internationalize bot-authored user-facing text through a central English message catalog with stable keys and named placeholders. Do not scatter display strings or assemble translated sentences in business logic.
- V1 ships English only. Keep localization extensible, with English as the default and fallback. Documentation, logs, and developer diagnostics remain in English.
