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

## Resolving Parallel Insertion Conflicts Across Virtual Branches

When two or more virtual branches insert lines at the exact same base anchor (e.g., adding imports, endpoints, or functions at the same location), add a comment-only `[gix-merge-insert-order: <i32>]` directive at the top of the inserted block instead of modifying the base commit:

- **Syntax**: Place `[gix-merge-insert-order: <i32>]` on a comment-only line (such as `// [gix-merge-insert-order: -10]`, `# [gix-merge-insert-order: 10]`, or `<!-- [gix-merge-insert-order: -10] -->`) as the first line of the inserted block.
- **Default order**: Unmarked insertions have an implicit order of `0`.
- **Ordering rules**:
  - Smaller integers are placed **before** larger integers at the same base anchor (e.g., use `-10` to insert above an existing unmarked (`0`) branch insertion, or `10` to insert below it).
  - You only need to add the directive to one of the conflicting branches (or to the uncommitted worktree insertion) if the other branch uses the implicit `0` order.
  - Multiple directives inside a single insertion hunk split that hunk into separately ordered segments.
  - Duplicate or inverted orders at the same boundary (e.g., `0` vs implicit `0`, or placing `10` above `0`) remain locked/conflicted in both `gix-merge` and `but-hunk-dependency`.

## Scoped Instructions

- For Rust work under `crates/`, follow `crates/AGENTS.md`.
- For desktop work under `apps/desktop/`, follow `apps/desktop/AGENTS.md`.
- For the React component library under `packages/but-ui-react/`, follow
  `packages/but-ui-react/AGENTS.md`.
