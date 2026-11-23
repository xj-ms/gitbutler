# GitButler (Fork)

This repository is a fork of [gitbutlerapp/gitbutler](https://github.com/gitbutlerapp/gitbutler), currently based on **0.23.3**.

Each feature or fix in this fork is maintained as an independent GitButler virtual branch (`fork/*`). The default branch (`HEAD`) is the **GitButler Workspace Commit**—an octopus merge of all applied virtual branches listed below—so cloning or building the default branch includes all fork changes combined while keeping each feature isolated on its own branch.

For the original upstream README, see [README.upstream.md](README.upstream.md).

## Changes in This Fork

### Features

- **Push Single Branch in a Stack** (`fork/push-single-branch`): Pushing a stacked branch in the desktop app or CLI (`but push <branch>`) now pushes only the selected branch without pushing unpushed parent branches below it, while `but push` (with no arguments) and `but pr new` still push all unpushed branches in the stack.
- **Push All Button for Workspace & Applied Branches** (`fork/push-all`): Added a **Push all** button in the desktop header (left of **Create branch**) with a confirmation modal to configure and persist `gitbutler/workspace`'s remote tracking branch and push `gitbutler/workspace` alongside any out-of-sync workspace base (branch or tag) and out-of-sync applied branches.
- **Update & Restore Pushed GitButler Workspaces** (`fork/update-pushed-workspace`): When `gitbutler/workspace`'s configured remote-tracking branch diverges from the local workspace and points to a valid managed `GitButler Workspace Commit`, the desktop header shows a workspace time-difference button with a confirmation modal to reset `gitbutler/workspace`, applied local branches, and base target metadata to match upstream; project setup also offers to restore a pushed GitButler workspace directly when selecting a branch that points to a managed workspace commit.
- **Switch Workspace Base Branch or Tag** (`fork/tag-as-base`): Initialize a project or switch your workspace target directly from the desktop app to a remote branch or Git tag (including from detached `HEAD`).
- **Update Branch Option for Amended Remote Commits** (`fork/update-amended-branch`): The **Update local branch...** action now appears when a remote branch was force-pushed with amended commits (same change ID, new commit SHA), alongside **Force push**.
- **Clean Merging of Adjacent Changes & Ordered Parallel Insertions** (`fork/parallel-adjacent-changes`): Parallel virtual branches that modify adjacent, non-overlapping lines in the same file now merge cleanly into the workspace without false conflicts, and parallel insertions at the same base line can be deterministically ordered using `[gix-merge-insert-order: <i32>]` comments.
- **Ignore Whitespace in Diffs** (`fork/ignore-whitespace-diffs`): Toggle a view-only "Ignore whitespace" mode in diff headers to hide whitespace-only changes and collapse whitespace-only hunks while reviewing changes.
- **Visual Studio Editor Support (Windows)** (`fork/vs-editor-support`): Select Visual Studio as your default external editor on Windows, opening files directly at the target line in the matching solution/window.
- **"Open with" Editor Submenus** (`fork/vs-editor-support`): Right-click files or diff hunks to open them in any supported editor via an **Open with** submenu, in addition to your configured default editor.
- **"Stage / Amend" Context Submenus** (`fork/submenu-stage-commit`): Right-click uncommitted files, folders, or diff hunks in the desktop app to stage or unstage them in any applied stack or amend them directly into a commit via a two-level **Stage / Amend** submenu (plus an **Amend with N selected lines** submenu when diff lines are selected), without needing drag-and-drop.
- **Highlight Matching Commit Files on Dependency Hover** (`fork/highlight-dependency-files`): Hovering a hunk dependency lock badge now highlights the corresponding file inside expanded commit file lists (in both list and tree views) in addition to the commit row.
- **GitHub Copilot SDK Local AI Provider** (`fork/copilot-ai-provider`): Use GitHub Copilot as a local AI provider in the desktop app, native GPUI client, and CLI (`but config ai copilot [--model <MODEL>]`), authenticating directly through your existing Copilot CLI login (`copilot login`, `gh auth login`, or environment tokens) with no API key setup.
- **Quick Project Removal in Project Switchers** (`fork/project-removal-actions`): Remove non-active projects directly from the header project dropdown or project switcher with a confirmation dialog, without opening Project Settings first.
- **Debug Console Toggle (Windows Debug Builds)** (`fork/tao-fix-and-debug-console`): Show or hide the dedicated debug terminal window from the **View** menu in Windows debug builds.

### Performance

- **Fast Status Checks After Undo / Snapshot Restore** (`fork/preserve-index-stat-cache`): Restoring an operation-log snapshot (`undo`/`redo`/restore) keeps the Git index stat cache warm for both untouched and newly restored files, avoiding slow re-scans and Git LFS filter passes on subsequent status checks.
- **Faster File & Hunk Assignment in Large Worktrees** (`fork/assign-diff-hunk-only`, `fork/assign-hunk-noop`): Staging or assigning files and hunks to a branch updates the UI immediately without a full worktree reload, diffs only the affected files when needed, and avoids redundant snapshot overhead.
- **Cached Commit Diffs for Dependency Calculation** (`fork/stack-commit-diff-cache`): Commit tree diffs are cached in memory across workspace updates, avoiding repeated diff calculations when stacks have not changed.
- **Performance Tracing for Staging & Amending** (`fork/assign-diff-hunk-only`, `fork/amend-telemetry`): Added detailed performance tracing spans across hunk assignment, dependency calculation, and commit amend/rebase flows to aid profiling.

### Bug Fixes

- **Refresh Stack Commits Immediately After Commit Edits & External Git Changes** (`fork/fix-stale-commit-sha`): Amending, uncommitting, squashing, rewording, or moving files between commits—as well as external Git branch updates on Windows—now immediately updates stack lanes with the new commits instead of leaving stale commits that cause follow-up actions to fail with *"Failed to find commit ... in rebase editor"*.
- **Fix Empty Window When Opening Project in New Window on Windows** (`fork/fix-open-project-window`): Opening a project in a new window on Windows no longer hangs with a blank window.
- **Persist Stack Lane Order Across Refreshes & View Switches** (`fork/fix-stack-order`): Reordering stack lanes in the workspace view now persists reliably across branch moves, tear-offs, and view switches without triggering a full workspace reload.
- **Prevent Out-of-Memory Webview Crashes** (`fork/fix-query-hook-leak`): Fixed a frontend subscription leak (especially noticeable in workspaces with locked files) that caused memory usage to grow continuously until the renderer crashed.
- **Reliable Edit-Mode Checkout on Windows** (`fork/checkout-skip-locked-files`): Saving and exiting edit mode no longer aborts halfway when another process (such as a terminal, IDE, or language server) holds an open handle to a deleted file's parent directory.
- **Fix Windows Input-Lock Deadlock** (`fork/tao-fix-and-debug-console`): Backported the upstream `tao` fix to prevent window freezes caused by a Windows input-lock deadlock.
- **Preserve Unminified Frontend in Debug Builds** (`fork/tauri-env-debug`): Debug desktop builds now retain unminified frontend bundles when `TAURI_ENV_DEBUG` is enabled, preserving readable stack traces and heap profiles.
