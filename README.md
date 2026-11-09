# GitButler (Fork)

This repository is a fork of [gitbutlerapp/gitbutler](https://github.com/gitbutlerapp/gitbutler), currently based on **0.22.3**.

Each feature or fix in this fork is maintained as an independent GitButler virtual branch (`fork/*`). The default branch (`HEAD`) is the **GitButler Workspace Commit**—an octopus merge of all applied virtual branches listed below—so cloning or building the default branch includes all fork changes combined while keeping each feature isolated on its own branch.

For the original upstream README, see [README.upstream.md](README.upstream.md).

## Changes in This Fork

### Features

- **Push All Button for Workspace & Applied Branches** (`fork/push-all`): Added a **Push all** button in the desktop header (left of **Create branch**) with a confirmation modal to configure and persist `gitbutler/workspace`'s remote tracking branch and push `gitbutler/workspace` alongside all out-of-sync applied branches.
- **Switch Workspace Base Branch or Tag** (`fork/tag-as-base`): Initialize a project or switch your workspace target directly from the desktop app to a remote branch or Git tag (including from detached `HEAD`).
- **Confirm Before Unapplying Conflicting Stacks** (`fork/confirm-conflicting-stacks`): When applying a branch that conflicts with active workspace branches, GitButler now shows a confirmation modal listing the affected stacks and conflicting files before unapplying them.
- **Update Branch Option for Amended Remote Commits** (`fork/update-amended-branch`): The **Update local branch...** action now appears when a remote branch was force-pushed with amended commits (same change ID, new commit SHA), alongside **Force push**.
- **Clean Merging of Adjacent Changes Across Parallel Branches** (`fork/parallel-adjacent-changes`): Parallel virtual branches that modify adjacent, non-overlapping lines in the same file now merge cleanly into the workspace without false conflicts.
- **Ignore Whitespace in Diffs** (`fork/ignore-whitespace-diffs`): Toggle a view-only "Ignore whitespace" mode in diff headers to hide whitespace-only changes and collapse whitespace-only hunks while reviewing changes.
- **Visual Studio Editor Support (Windows)** (`fork/vs-editor-support`): Select Visual Studio as your default external editor on Windows, opening files directly at the target line in the matching solution/window.
- **"Open with" Editor Submenus** (`fork/vs-editor-support`): Right-click files or diff hunks to open them in any supported editor via an **Open with** submenu, in addition to your configured default editor.
- **Highlight Matching Commit Files on Dependency Hover** (`fork/highlight-dependency-files`): Hovering a hunk dependency lock badge now highlights the corresponding file inside expanded commit file lists (in both list and tree views) in addition to the commit row.
- **Quick Project Removal in Project Switchers** (`fork/project-removal-actions`): Remove non-active projects directly from the header project dropdown or project switcher with a confirmation dialog, without opening Project Settings first.
- **Debug Console Toggle (Windows Debug Builds)** (`fork/tao-fix-and-debug-console`): Show or hide the dedicated debug terminal window from the **View** menu in Windows debug builds.

### Performance

- **Preserve `.git/index` Stat Cache Across Oplog Snapshot Restores** (`fork/preserve-index-stat-cache`): Restoring an operation-log snapshot (`undo`/`redo`/restore) now preserves valid `.git/index` `stat` metadata instead of zeroing out every entry via `index_from_tree`, preventing `gix::status` from re-hashing and re-running clean filters (such as `git-lfs filter-process`) across the entire worktree on every subsequent status check.
- **Faster File & Hunk Assignment in Large Worktrees** (`fork/assign-diff-hunk-only`, `fork/assign-hunk-noop`): Staging or assigning files to a branch updates the frontend worktree cache in place without triggering a full `changes_in_worktree` reload, skips `gix` worktree diffing when requested hunks already match persisted state (falling back to scoped per-path diffing otherwise), skips computing unified patches for worktree files without committed dependency ranges, and avoids a redundant ~1.15s operation-log snapshot.
- **Cached Commit Diffs for Dependency Calculation** (`fork/stack-commit-diff-cache`): Commit tree diffs are cached in memory across workspace watcher updates, avoiding repeated Git object lookups and diff computations when stacks have not changed.
- **Performance Tracing for Staging & Amending** (`fork/assign-diff-hunk-only`, `fork/amend-telemetry`): Added detailed tracing spans across hunk assignment, dependency calculation, and commit amend/rebase paths to aid profiling.

### Bug Fixes

- **Persist Stack Lane Order Across Refreshes & View Switches** (`fork/fix-stack-order`): Reconciled workspace metadata with projected stacks in `update_stack_order_with_perm` so reordered lanes persist even after branch moves or tear-offs, and updated the frontend `workspaceDetails` cache in place without triggering a full workspace reload.
- **Prevent Out-of-Memory Webview Crashes** (`fork/fix-query-hook-leak`): Fixed a reactive query subscription leak in the desktop UI (especially noticeable with locked files) that accumulated thousands of hooks over time and caused the renderer to crash near the 4 GB heap limit.
- **Reliable Edit-Mode Checkout on Windows** (`fork/checkout-skip-locked-files`): Saving and exiting edit mode no longer aborts halfway when another process (such as a terminal, IDE, or language server) holds an open handle to a deleted file's parent directory.
- **Fix Windows Input-Lock Deadlock** (`fork/tao-fix-and-debug-console`): Backported the upstream `tao` fix to prevent window freezes caused by a Windows input-lock deadlock.
- **Preserve Unminified Frontend in Debug Builds** (`fork/tauri-env-debug`): Passed `TAURI_ENV_DEBUG` through Turborepo so debug desktop builds retain unminified frontend bundles when requested.
