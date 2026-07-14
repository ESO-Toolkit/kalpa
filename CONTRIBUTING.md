# Contributing

Thanks for your interest in contributing to Kalpa!

## Getting Started

1. Fork the repository
2. Clone your fork and install dependencies:

```bash
git clone https://github.com/YOUR_USERNAME/kalpa.git
cd kalpa
npm install
```

3. Create a `.env.local` file with the dev server port:

```bash
echo "VITE_PORT=1430" > .env.local
```

4. Start the development server:

```bash
npm run tauri dev
```

The Vite dev server runs on port **1430** (configured in `.env.local` and `src-tauri/tauri.conf.json`).

## Prerequisites

- [Rust](https://rustup.rs/) (stable, MSVC toolchain on Windows)
- [Node.js](https://nodejs.org/) 22+
- On Windows: Visual Studio Build Tools with "Desktop development with C++" and the [WebView2](https://developer.microsoft.com/en-us/microsoft-edge/webview2/) runtime (pre-installed on Windows 11)
- On macOS: Xcode Command Line Tools (`xcode-select --install`)
- On Linux (Debian/Ubuntu): `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev patchelf libssl-dev libxdo-dev build-essential`

Run `npm run check:env` to verify your prerequisites are properly configured.

Note: the Playwright E2E suite is currently Windows-only (it drives the app over CDP, which only WebView2 exposes — see `playwright.config.ts`). Unit tests (`npx vitest run`, `cargo test`) run on all platforms.

## Development Workflow

1. Create a branch from `main`: `git checkout -b feat/your-feature` or `fix/your-bug`
2. Make your changes
3. Run checks before committing:

```bash
# Frontend
npm run check        # tsc + eslint + prettier

# Backend
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

4. Commit using [Conventional Commits](https://www.conventionalcommits.org/):
   - `feat: add new feature`
   - `fix: resolve bug`
   - `refactor: restructure code`
   - `docs: update documentation`
5. Open a pull request against `main`

## Pull Requests

- Keep PRs focused — one feature or fix per PR
- Ensure all CI checks pass before requesting review
- Fill out the PR template (What / Why / How / Testing)
- Expect a review within a few days; maintainers may request changes
- Squash-merge is preferred for a clean history

## Code Style

- **TypeScript**: Strict mode, no `any` types, Prettier formatting
- **Rust**: `cargo fmt` + `cargo clippy` with `-D warnings`
- **CSS**: Tailwind v4 utility classes, follow the design system in `context/40-design-system.md`

## Architecture

See the project structure in the README and detailed docs in the `context/` directory:

- `context/00-overview.md` — Core vision and principles
- `context/10-desktop-client.md` — Desktop client architecture
- `context/40-design-system.md` — Design system and styling rules

## Reporting Issues

- Use [GitHub Issues](https://github.com/ESO-Toolkit/kalpa/issues)
- Include steps to reproduce, expected vs actual behavior
- For security vulnerabilities, see [SECURITY.md](SECURITY.md)
