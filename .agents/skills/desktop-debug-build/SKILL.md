---
name: desktop-debug-build
description: Use when asked to build (or rebuild) the debug GitButler desktop app (`target\debug\gitbutler-tauri.exe`) with a bundled frontend, e.g. to verify a frontend or backend fix outside of `pnpm dev:desktop`.
---

## What the "debug build" is

- `gitbutler-tauri` enables the `custom-protocol` feature by default, so even a plain
  `cargo build` produces a binary that serves the **bundled** frontend from
  `http://tauri.localhost` instead of the Vite dev server.
- The frontend is embedded at compile time from `apps/desktop/build`
  (`frontendDist` in `crates/gitbutler-tauri/tauri.conf.json`).
- Therefore: **build the frontend first, then the Rust binary.** A Rust build alone
  embeds whatever stale `apps/desktop/build` is on disk.

## Steps

Run from the repo root. The app must not be running (the `.exe` is locked while it runs).

1. Build the frontend in development mode, **unminified** (matches the `beforeBuildCommand` in
   `tauri.conf.json`, which can't run under Windows `cmd`):

   ```powershell
   $env:TAURI_ENV_DEBUG = "true"
   pnpm build:desktop '--' --mode development
   ```

   - `TAURI_ENV_DEBUG` turns off minification (`build.minify` in `apps/desktop/vite.config.ts`).
     That gives readable function and variable names in stack traces, heap snapshots and profiles.
     Leave it unset for a minified bundle. It's listed in `turbo.json` `env`, so turbo passes it
     through and caches the two variants separately.
   - In PowerShell the `--` **must be quoted**. A bare `--` is swallowed, and turbo then fails
     with `unexpected argument '--mode' found`.
   - In bash: `TAURI_ENV_DEBUG=true pnpm build:desktop -- --mode development`.
   - Takes ~1–2 minutes. The Sentry "No auth token provided" warnings are expected.
   - Check `apps/desktop/build` has a fresh timestamp.

2. Build the Rust binary:

   ```powershell
   cargo build -p gitbutler-tauri
   ```

   Output: `target\debug\gitbutler-tauri.exe`. It also needs `target\debug\gitbutler-git-askpass.exe`
   (and `but.exe` for CLI features). If they are missing, run `cargo build` (whole workspace) once.

3. Verify the new frontend is embedded (optional, but cheap). Pick a chunk file name from
   `apps\desktop\build\_app\immutable\chunks\` that contains your change and check that the exe
   references it:

   ```powershell
   $b = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes("target\debug\gitbutler-tauri.exe"))
   $b.Contains('<chunk-name>.js')
   ```

   If it's `False`, Cargo didn't notice the frontend change. Touch `crates\gitbutler-tauri\build.rs`
   and rebuild.

## Running and debugging

- Launch `target\debug\gitbutler-tauri.exe`. It uses the `com.gitbutler.app.dev` identifier.
- DevTools: right-click → Inspect, or `Ctrl+Shift+I` (the `devtools` feature is on by default).
- Logs: `%LOCALAPPDATA%\com.gitbutler.app.dev\logs\` (`GitButler.<date>.log` for the backend,
  `ui-logs.log` / `GitButler Dev.log` for forwarded webview errors).
- Webview crash dumps (e.g. renderer OOM): `%LOCALAPPDATA%\com.gitbutler.app.dev\EBWebView\Crashpad\reports\`.
- Remote DevTools protocol, e.g. for scripted heap snapshots: set
  `$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222"` before launching.
- Source maps are always emitted (`build.sourcemap: true`) and embedded, so DevTools can map
  frames back to `.svelte`/`.ts` sources even for a minified build.
- For hot reload, use `pnpm dev:desktop` instead (Vite dev server; builds into `target\tauri`).
