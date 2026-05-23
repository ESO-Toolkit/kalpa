# What's scrubbed in `.esopack` v2

`.esopack` v2 packs can optionally bundle **account-wide addon settings** so
you can share a fully-configured setup, not just a list of addons. Because ESO
SavedVariables files contain personal data, Kalpa automatically scrubs them
before anything is written to the pack file, and re-maps the result to the
person who imports it.

This page documents exactly what that scrubbing does so you can decide what is
safe to share.

## How it works

When you export settings into a pack, Kalpa:

1. **Reads each addon's SavedVariables** from your AddOns SavedVariables folder.
2. **Detects your identities** — your account handle(s), character names,
   character IDs, and megaserver names — by recognizing ESO's standard
   SavedVariables layout.
3. **Scrubs the data** (see below).
4. **Keeps only account-wide settings.** Per-character subtrees are stripped
   entirely in this release — only the `$AccountWide` portion of each addon is
   exported.
5. **Serializes the cleaned result** into the pack.

On import, the placeholders are substituted with **your own** account handle,
characters, and megaserver, so shared settings apply to your account rather
than the author's.

## What is removed

- **Your identity in keys** is replaced with placeholders (`${ACCOUNT}`,
  `${CHAR:N}`, `${CHAR_ID:N}`, `${WORLD}`) and re-mapped on import — your real
  account handle, character names, character IDs, and megaserver names never
  appear in the file.
- **Identity-bearing values are dropped outright** (not templated): any string
  value containing your account handle or a character name/ID, and anything
  matching the `@Handle` shape. This catches ignore lists, whisper targets, and
  similar that hold other players' handles.
- **Data-heavy / social collections are dropped** when their table key name
  indicates personal data — for example: mail, friends, whisper logs, sales and
  purchase/trade history, guild store/history/bank/roster tables, bank and
  inventory/bag contents, gold/currency/wallet, "recent"/"last seen"/"last
  online" tables, combat logs and fight data, event/message logs, per-session
  chat logs, and character-id↔name lookup tables.
- **Identity helper keys** such as `$LastCharacterName`, `lastCharname`, and
  `charName` are always dropped.

## What is kept

- **Your actual addon configuration** — booleans, numbers, colors, modes, UI
  layouts and positions, and addon-specific toggles in the account-wide section.
- Config that merely *looks* like it might be social but isn't, such as an
  ability ignore-list (`ignoredAbilities`), is preserved.

## Caveats

- **Account-wide only.** Per-character settings are not exported in this
  release, so character-specific tweaks won't transfer.
- **The scrubber is heuristic and conservative.** It targets ESO's standard
  SavedVariables shapes and known personal-data patterns. If an addon stores
  sensitive data (for example, an external-service token) under an unusual key
  the heuristics don't recognize, it could be retained. **Treat exported packs
  as you would any file you publish: review before sharing widely**, especially
  for addons that integrate with external services.
- **Re-mapping is best-effort.** Placeholders map to the identities the import
  detects on your machine; addons with non-standard layouts may not re-map
  perfectly and can be re-configured in-game.

If you find an addon whose settings aren't scrubbed or re-mapped correctly,
please [open an issue](https://github.com/ESO-Toolkit/kalpa/issues) so the
rules can be tuned.
