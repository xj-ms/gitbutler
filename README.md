# GitButler (Fork)

This repository is a fork of [gitbutlerapp/gitbutler](https://github.com/gitbutlerapp/gitbutler), currently based on **0.22.3**.

For the original upstream README, see [README.upstream.md](README.upstream.md).

## Changes in This Fork

### Features

- **Switch Workspace Base Branch or Tag**: Switch your workspace target directly from the desktop app to another remote branch or tag without manually reconfiguring the project.
- **Confirm Before Unapplying Conflicting Stacks**: When applying a branch that conflicts with active workspace branches, GitButler now shows a confirmation modal listing the affected stacks and conflicting files before unapplying them.
- **Update Branch Option for Amended Remote Commits**: The **Update local branch...** action now appears when a remote branch was force-pushed with amended commits (same change ID, new commit SHA), alongside **Force push**.
- **Clean Merging of Adjacent Changes Across Parallel Branches**: Parallel virtual branches that modify adjacent, non-overlapping lines in the same file now merge cleanly into the workspace without false conflicts.
- **Ignore Whitespace in Diffs**: Toggle a view-only "Ignore whitespace" mode in diff headers to hide whitespace-only changes and collapse whitespace-only hunks while reviewing changes.
- **Visual Studio Editor Support (Windows)**: Select Visual Studio as your default external editor on Windows, opening files directly at the target line in the matching solution/window.
- **"Open with" Editor Submenus**: Right-click files or diff hunks to open them in any supported editor via an **Open with** submenu, in addition to your configured default editor.
- **Highlight Matching Commit Files on Dependency Hover**: Hovering a hunk dependency lock badge now highlights the corresponding file inside expanded commit file lists (in both list and tree views) in addition to the commit row.
- **Quick Project Removal in Project Switchers**: Remove non-active projects directly from the header project dropdown or project switcher with a confirmation dialog, without opening Project Settings first.
- **Debug Console Toggle (Windows Debug Builds)**: Show or hide the dedicated debug terminal window from the **View** menu in Windows debug builds.

### Performance

- **Faster File & Hunk Assignment in Large Worktrees**: Staging or assigning files to a branch now diffs and reconciles only the affected file paths instead of re-diffing every modified file in the worktree, and skips a redundant ~1.15s operation-log snapshot.
- **Cached Commit Diffs for Dependency Calculation**: Commit tree diffs are cached in memory across workspace watcher updates, avoiding repeated Git object lookups and diff computations when stacks have not changed.
- **Performance Tracing for Staging & Amending**: Added detailed tracing spans across hunk assignment, dependency calculation, and commit amend/rebase paths to aid profiling.

### Bug Fixes

- **Prevent Out-of-Memory Webview Crashes**: Fixed a reactive query subscription leak in the desktop UI (especially noticeable with locked files) that accumulated thousands of hooks over time and caused the renderer to crash near the 4 GB heap limit.
- **Reliable Edit-Mode Checkout on Windows**: Saving and exiting edit mode no longer aborts halfway when another process (such as a terminal, IDE, or language server) holds an open handle to a deleted file's parent directory.
- **Fix Windows Input-Lock Deadlock**: Backported the upstream `tao` fix to prevent window freezes caused by a Windows input-lock deadlock.
- **Preserve Unminified Frontend in Debug Builds**: Passed `TAURI_ENV_DEBUG` through Turborepo so debug desktop builds retain unminified frontend bundles when requested.
