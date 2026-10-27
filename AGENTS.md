# GitButler Agent Instructions

GitButler is a Rust/Svelte/React/TypeScript monorepo.

Apply all relevant instruction files. If instructions conflict, resolve them in
this order:

1. Explicit human instructions
2. Nearest nested `AGENTS.md`
3. This file

## Repo Map

- `crates/` - Rust crates.
- `apps/desktop/` - Tauri/Svelte desktop app.
- `apps/web/` - Svelte web app.
- `packages/` - shared TypeScript packages, including the SDK.
- `e2e/` - Playwright, WebdriverIO, and blackbox end-to-end tests.

## Working Style

- Treat questions about the codebase as read-only unless the user asks for changes.
- Make focused, reviewable changes; avoid unrelated rewrites.
- Use the simplest design that solves the actual problem; do not add
  speculative machinery, and remove machinery your change makes unnecessary.
- Inspect nearby code before introducing patterns.
- Prefer existing APIs, tests, and conventions.
- Before declaring shared behavior done, check each applicable surface and contract
  (desktop, web, CLI/TUI, SDK types, and docs) and update it or explicitly
  determine that it is unaffected.
- Run targeted validation for the area touched.
- Before adding new machinery to fix a behavior bug, reproduce the bug in a failing
  test and survey the target file's existing loops and classifications as candidate
  hosts; let the tests, not the diagnosis, set how much implementation the fix needs.
- When a fix calls for a new mechanism (a new module, a new public API, or a parallel
  walk where one already exists), propose the intended shape before building it.

## GitButler (`but`) CLI in This Repository

- When running `but` commands against this repository, always use the `but` binary built from this repository (`.\target\debug\but.exe`, built via `cargo build -p but`) instead of a globally installed `but` on `PATH`.
- This fork's workspace relies on fork-specific behavior (such as the `gix-merge` adjacent-line merge patch), so stock `but` releases on `PATH` may fail to merge workspace bases or use incompatible metadata defaults.

## Scoped Instructions

- For Rust work under `crates/`, follow `crates/AGENTS.md`.
- For desktop work under `apps/desktop/`, follow `apps/desktop/AGENTS.md`.
- For the React component library under `packages/but-ui-react/`, follow
  `packages/but-ui-react/AGENTS.md`.
