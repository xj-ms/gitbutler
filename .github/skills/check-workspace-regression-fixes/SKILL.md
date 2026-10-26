---
name: check-workspace-regression-fixes
description: >-
  Check whether the latest upstream GitButler default-branch head fixes the two
  workspace regressions investigated locally: unapply failing while rebuilding
  the workspace merge after metadata reconciliation changes, and upstream
  integration failing to replay a managed multi-parent workspace commit with
  "Failed to merge bases while cherry picking". Use when asked whether latest
  upstream or master fixes either issue. Verifies live GitHub state, inspects
  the actual source and tests, and distinguishes partial fixes from fixes
  covering the reproduced scenarios without changing local Git refs or the
  working tree.
---

# Check workspace regression fixes

Use this skill to re-check two specific GitButler regressions against live
upstream. Report only on the current default-branch head. Do not rely on the
local `origin/master`, remembered PR state, or an issue being closed.

## Regressions being tracked

### 1. Unapply workspace-rebuild regression

Known behavior:

- Official `0.20.4` could unapply `refs/heads/user/jing/vpack` in the reproduced
  `D:\WPA` workspace.
- A later build failed with:

  ```text
  Failed to unapply branch due to conflicts while rebuilding the workspace merge commit: loading-reason
  ```

- The suspected behavioral change is commit
  `8e7cde6a0a4d5a9cb60e2cec7f8d0b9d22c1bdac`, merged by #14890 and first
  released in `0.22.1`.
- In `crates/but-workspace/src/branch/unapply.rs`, the old implementation
  reconciled projected stacks explicitly with `reconcile_projected_stacks()`.
  The suspected version calls `ws.reconcile_metadata(&mut ws_md)`.

Treat this cause as a strong hypothesis, not a proven diagnosis. A valid
upstream fix needs evidence that it addresses the post-reconciliation set or
ordering of workspace tips, the rebuilt workspace merge, or the exact unapply
failure. A rename, refactor, or generic metadata cleanup is not sufficient.

### 2. Managed multi-parent workspace replay regression

Known behavior:

- Official `0.20.4` successfully previewed upstream integration in the
  reproduced workspace.
- `0.21.0` and later failed while replaying managed workspace commit
  `305f71eb40c8c917945ea1adcff8c0c9bba8909c`:

  ```text
  Failed to merge bases while cherry picking commit 305f71eb...
  ```

- The commit has ten original parents, and all ten parents are replaced during
  the failing integration preview.
- The original and replacement parent sets both fail generic parent-tree
  reconstruction.
- The first reproduced original-parent conflict is:

  ```text
  analyzer/UI.DataEngine/hdvviewmodel.cs
  ```

  One parent changes `PrecomputeValuesNecessaryForGraphing()` from `bool` to
  `void`; another adds the `"Preparing graph..."` operation description while
  retaining `return true`. The stored workspace tree contains the valid
  combination.
- The regression was introduced by commit
  `38ce650c6aa80f217e097066738edb643dd602f3`, part of #14275 and first
  released in `0.21.0`. It made the editor include every graph segment, causing
  the managed workspace commit to enter the generic replay path.
- #15112 is related.
- #15123 is a conservative proposed fallback for exactly one rewritten parent.
  It explicitly declines multiple rewritten parents, so that version of the PR
  does **not** fix this ten-parent reproduction.

A complete fix for this reproduction must do at least one of the following:

- avoid replaying the managed workspace commit as an ordinary merge commit;
- rebuild it with workspace-specific graph/stack semantics;
- safely transform its recorded tree when multiple parents are rewritten; or
- otherwise demonstrate, with a multi-parent regression test, that the ten-parent
  class no longer reaches `FailedToMergeBases`.

A single-parent-only fallback is a **partial related fix**, not a fix for this
issue.

## Safety and source-of-truth rules

- Use `gh` against `gitbutlerapp/gitbutler` for live GitHub state.
- Do not run `git fetch`, checkout another revision, reset, stash, merge, rebase,
  or otherwise modify local refs, the index, or the working tree.
- Do not run an integration mutation against a user's repository.
- Prefer GitHub file contents at the resolved upstream SHA over local files.
- Use current repository files only to understand context or compare known local
  instrumentation; never assume they represent latest upstream.
- Do not claim a fix solely from a PR title, issue status, or code comment.
  Inspect implementation and tests at the resolved head.
- If GitHub access fails, report that the live result could not be verified.

## Workflow

### 1. Resolve the actual latest upstream revision

Get the default branch and its current commit:

```powershell
$repo = "gitbutlerapp/gitbutler"
$branch = gh repo view $repo --json defaultBranchRef --jq ".defaultBranchRef.name"
$latest = gh api "repos/$repo/commits/$branch" --jq ".sha"
gh api "repos/$repo/commits/$latest" --jq "{sha: .sha, date: .commit.committer.date, subject: .commit.message}"
```

Record the full SHA and commit date in the result. Never describe a cached local
revision as "latest".

### 2. Search live issues, PRs, and commits

Search separately for each symptom and mechanism. Include both issues and PRs:

```powershell
gh search issues '"Failed to unapply branch due to conflicts"' --repo $repo --include-prs --limit 50
gh search issues '"conflicts while rebuilding the workspace merge commit"' --repo $repo --include-prs --limit 50
gh search issues 'unapply workspace merge reconcile metadata' --repo $repo --include-prs --limit 50

gh search issues '"Failed to merge bases while cherry picking"' --repo $repo --include-prs --limit 50
gh search issues '"managed workspace commit" replay' --repo $repo --include-prs --limit 50
gh search issues 'multi-parent workspace merge' --repo $repo --include-prs --limit 50
```

Also inspect:

- #14890
- #15112
- #15123

For relevant source paths, query commits on the current default branch:

```powershell
gh api "repos/$repo/commits?sha=$branch&path=crates/but-workspace/src/branch/unapply.rs&per_page=100"
gh api "repos/$repo/commits?sha=$branch&path=crates/but-rebase/src/graph_rebase/cherry_pick.rs&per_page=100"
gh api "repos/$repo/commits?sha=$branch&path=crates/but-rebase/src/graph_rebase/rebase.rs&per_page=100"
gh api "repos/$repo/commits?sha=$branch&path=crates/but-rebase/src/graph_rebase/workspace_commit.rs&per_page=100"
```

If a candidate fix is found, inspect its PR body, complete diff, review
discussion, merge status, and tests:

```powershell
gh pr view <number> --repo $repo --json number,title,state,isDraft,mergedAt,updatedAt,url,body,commits,files,reviews
gh pr diff <number> --repo $repo --patch --color never
gh api "repos/$repo/pulls/<number>/comments" --paginate
```

### 3. Read source at the resolved upstream SHA

Fetch files directly from GitHub. Decode the Base64 `content` field before
inspection:

```powershell
function Get-UpstreamFile([string]$path) {
    $content = gh api --method GET "repos/$repo/contents/$path" -f ref=$latest --jq ".content"
    [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String(($content -replace "\s", "")))
}
```

Inspect at minimum:

```text
crates/but-workspace/src/branch/unapply.rs
crates/but-workspace/src/commit/mod.rs
crates/but-api/src/legacy/virtual_branches.rs
crates/but-rebase/src/graph_rebase/cherry_pick.rs
crates/but-rebase/src/graph_rebase/rebase.rs
```

If present, also inspect:

```text
crates/but-rebase/src/graph_rebase/workspace_commit.rs
```

Search tests and fixtures for the exact behaviors:

```text
conflicts while rebuilding the workspace merge commit
FailedToMergeBases
replay_single_parent_delta
multiple rewritten parents
managed workspace commit
multi-parent
octopus
```

### 4. Classify the unapply issue

Use one of these classifications:

- **Fixed with direct evidence**: upstream includes a targeted semantic change
  and a test reproducing the unapply/workspace-tip conflict class.
- **Likely fixed**: a targeted semantic change clearly addresses the suspected
  tip reconciliation problem, but no exact regression test exists.
- **Still present**: the suspected path is unchanged and no relevant fix exists,
  or a current test/source analysis shows the failure remains possible.
- **Inconclusive**: the implementation changed substantially and source review
  cannot establish behavior without a reproduction.

Explicitly state whether `ws.reconcile_metadata()` remains, but do not use its
presence alone as proof that the bug remains. The fix may be elsewhere in tip
selection, metadata reconciliation, or workspace commit construction.

### 5. Classify the workspace replay issue

Count how many rewritten parents a candidate fix supports.

- If the code requires exactly one changed parent, classify it as **partial;
  does not cover the reproduced ten-parent case**.
- If it accepts multiple changed parents, inspect how it orders and applies
  changes, handles renames, and surfaces real conflicts.
- If it rebuilds via workspace metadata or excludes the managed workspace commit
  from generic replay, verify the relevant invariant with tests.
- Look for a test with at least three independently rewritten workspace parents.
  A two-parent test with one unchanged parent is not enough evidence for the
  reproduced class.

Use one of:

- **Fixed for the reproduced multi-parent case**
- **Partial fix only**
- **Still present**
- **Inconclusive**

### 6. Present a compact evidence table

Use this shape:

| Regression | Latest upstream head | Evidence |
|---|---|---|
| Unapply workspace rebuild | Fixed / Likely fixed / Still present / Inconclusive | Commit, source behavior, and test |
| Multi-parent workspace replay | Fixed / Partial only / Still present / Inconclusive | Parent-count support and regression tests |

Then include:

- the exact upstream SHA and date checked;
- the most relevant PRs/issues;
- any remaining uncertainty;
- whether testing a local build against a disposable clone would materially
  increase confidence.

Do not report release availability or recommend a release. If only one
regression is fixed at the latest head, say so prominently.
