# Kalpa — Codex Guide

You are Codex working in this repository. Optimize for **safety, clarity, and maintainability** while helping evolve this project.

## Read `CLAUDE.md` first — all of it applies to you

`CLAUDE.md` is the single source of truth for this repo: mission and current state, tech stack, core constraints, project structure, the Pack Hub worker rules, code-quality gates, git workflow and the release process, the design system, and the dev-server port.

Everything in it applies to Codex exactly as written. Substitute "Codex" wherever it says "Claude".

This file deliberately holds no copy of that guidance. It used to, and the copy drifted: it still taught the three-file version bump that shipped nine consecutive tags with a stale `package-lock.json`, an always-dark visual system that the light themes replaced, and a debugging flow pointing at a port the dev server does not use. A pointer cannot go stale.

## Codex-specific notes

- Codex's own workspace lives in `.codex/` and is gitignored; do not commit anything from it.
- Project MCP servers are configured in `.mcp.json` at the repo root.
