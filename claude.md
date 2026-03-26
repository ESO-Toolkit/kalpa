\# ESO Addon Manager



You are Claude Code working in this repo.



Goal: build a proper open-source ESO addon manager with:

\- A desktop client that installs directly into the ESO AddOns folder

\- Automatic dependency resolution from addon manifests

\- Update checks from public ESOUI pages

\- A minimal metadata-only backend if needed

\- Near-zero recurring cost

\- A maintainable architecture that can scale to hundreds or thousands of users



\## Important rules



\- Do not use private APIs or hacks.

\- Prefer public ESOUI pages and direct public download URLs.

\- Keep scraping centralized and cached.

\- Do not implement hourly background scraping.

\- Prefer on-open refresh plus a manual Refresh button.

\- Optimize for maintainability and simplicity over cleverness.



\## Preferred stack



\- Desktop client: Tauri + React + TypeScript

\- Backend: Cloudflare Workers + KV, metadata only



\## Available tools



\- You may use `gh` to create and push the repo.

\- You may use `wrangler` to create and deploy the Cloudflare Worker.

\- You may use the local Rust/Node toolchain as needed.



\## How to work



1\. Read `context/00-overview.md`

2\. Read the task file relevant to the current phase

3\. Make small, reviewable changes

4\. Keep the repo buildable after each phase

5\. Ask before making large architecture changes



\## First task



Start by scaffolding the repo structure and implementing the MVP plan in `context/30-mvp-plan.md`.



