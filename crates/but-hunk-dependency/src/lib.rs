#![deny(missing_docs)]

//! ## Module Guidelines
//!
//! ### Data for consumption by UI
//!
//! Data-types for the user-interface are colocated in a `ui` module close to the module where the plumbing-type is located.
//!
//! ## A possible future
//!
//! What follows is research on how one could implement a perfectly *accurate* version of the existing algorithm that *doesn't* use patch context lines,
//! while producing the result-blobs for each commit as needed 'automatically'.
//!
//! For now, this crate just ports `gitbutler-hunk-dependency` to `gix` types.
//!
//! ### Terminology
//!
//! * **WorktreeHunk**
//!     - A patch if applied to `HEAD^{tree}` would turn that resource into the `WorktreeState`.
//! * **CommitHunk**
//!     - A patch generated from a commit and its parent, indicating the change that the commit represents.
//!     - If there are multiple parents, only the first one is used for obtaining CommitHunks.
//! * **WorktreeState**
//!     - A file at a `Path` as it would be found in the *worktree*.
//!     - If that file is compared to the `HEAD^{tree}` we get `WorktreeHunks`.
//! * **CommitState**
//!     - A *Blob* in the *Git tree* at a `Path`.
//! * **Blob**
//!     - The bytes of a file, ready for storage in Git.
//! * **BranchTip**
//!     - The top-most commit in a Git branch.
//! * **BranchBase**
//!     - The floor of a Git branch, which itself isn't considered part of the branch anymore.
//!     - The *base* is used to compute a `CommitHunk` with its direct descendant commit, but its own `CommitHunk` is never used.
//! * **Branch**
//!     - A branch is all commits from a single `BranchTip` that is bounded by one or more `BranchBases`.
//!     - Just a Git branch.
//! * **Stack**
//!     - A list of `Branches` whose *commits* are naturally connected to each other, so the top-most `Branch` is connected with the bottom-most `Branch`.
//!     - These aren't represented directly here, as a `Stack` can be represented as `BranchTip` of the top-most branch to the `BranchBases` of the bottom-most branch,
//!       and we use the term `Branch` here for simplification instead of `Stack`.
//! * **BranchCommits**
//!     - The *commits* between the `BranchTip` and the `BranchBases`.
//! * **Workspace**
//!     - A set of `Stacks` which are all merged together into a single worktree, represented by a `WorkspaceCommit` that is an octopus merge between the `BranchTips` of all `Stacks`.
//! * **WorkspaceCommit**
//!     - The commit as the result of the octopus between the `BranchTips` of all `Stacks`.
//!     - Its tree is a merge of all `Stacks` and contains all their changes.
//! * **AmendableCommit**
//!     - A list of commits to which a `WorktreeHunk` cleanly applies without intersecting with any `CommitHunk`.
//! * **IntroducingCommit**
//!     - The first *commit* whose `CommitHunk` intersects with a `WorktreeHunk`. This means the hunk can override the overlapping portion of the `CommitHunk`
//!       and now knows the *last commit* (closest to `BranchBase`) that it can apply to without causing conflicts in future commits.
//!
//! ### Purpose
//!
//! This crate helps to associate one or more `WorktreeHunks` to one or more *commits* .
//! There are the following cases to consider, with varying levels of accuracy.
//! This algorithm is *state*-based and produces the `CommitState` for each `AmendableCommit` and `IntroducingCommit` so it contains all applicable `WorktreeHunks`.
//! It starts with the `WorktreeState` available, and the `CommitState` at `BranchTip` as well.
//! It's notable that even if commits would be amended with `WorkreeHunks`, the worktree itself does not change state at all.
//! (*Note that ContextLines aren't relevant here.*)
//!
//! ### Associate all `WorktreeHunks` to their `IntroducingCommits` in a `Workspace` TODO/Still unclear
//!
//! TODO: This *should* work with a blame-based-algorithm, as `git blame` can already do this. More testing required.
//!
//! A `Workspace` is the result of a merge of two or more `Branches`. This means its *worktree* is also the combination of two or more branches. If it is only one `Branch`,
//!
//! It seems easiest extract the `WorktreeHunks` (as `UnifiedDiff`) and then apply them one by one onto each candidate `Branch` in the `Workspace` with fuzzy matching
//! to find one that they apply to. When found, proceed with these patches similarly to how it's done with normal `Branches`.
//! This is probably helps with 80% of the `WorktreeHunks` that cleanly apply.
//! And then there are those that need to be split as they are partially in multiple `Branches`.
//!
//! Maybe another way to do this is to…
//!
//! - go through each `Branch`
//! - go through each `Commit` of a `Branch` from `BranchTip` to `BranchBases`
//! - merge in the `BranchTips` of the other `Branches` and cherry-pick the `WorktreeHunks` on top
//!
//! Essentially, perform the same algorithm as with simple `Branches`, but operate on a merge commit instead, simulating the effect of the `Workspace` at all times.
//! The problem here would be that it's very possible that the `Branches` don't merge cleanly in all cases.
//!
//! ### Associate all `WorktreeHunks` to their `IntroducingCommits` in `Branches`
//!
//! In standard Git `Branches`, the worktree matches the `BranchTip` and `WorktreeHunk` represent changes on top of that.
//! Here is an algorithm to associated `WorktreeHunks` with their `IntroducingCommits`.
//!
//! - prior to the walk, filter out all `WorktreeHunks` that aren't in any file that is touched by the `BranchCommits`.
//! - walk down from `BranchTip` to the `BranchBase`, and for each commit do a *three-way merge* such that we revert each commit, but pretend to have added `WorktreeHunk`
//!   at the same time. Alternatively, it's like cherry-picking the `WorktreeState` onto the parent of `BranchTip` as first iteration. Then it's like pushing `WorktreeHunk` down the
//!   commit-ancestry, starting at the `BranchTip` whose `State` we know with `WorktreeHunk` applied.
//!      - If there is a conflict, we know the clashing `CommitHunks` are to be superseded by the overlapping portions of the respective `WorktreeHunk`, which can be similar to choosing
//!        the *Ours* strategy. This removes the whole `WorktreeHunk` and if there are no more `WorktreeHunks` to track, we can stop iterating. This is the `IntroducingCommit` to record.
//!      - If the merge is without conflicts, we have the `State` of our side for use in the next iteration. Record this commit as `AmendableCommit`.
//!      - Keep iterating until all `WorktreeHunks` are associated with an `IntroducingCommit`.
//!      - Once the `BranchBase` is reached, stop the iteration
//! - All `WorktreeHunks` that were associated should be applied to the `BranchTip`, adding it as `AmendableCommit`
//! - `WorktreeHunks` that were *not* associated are returned and can be committed separately, for instance on top of the `BranchTip` whose `State` we have returned as well.
//!
//! The algorithm should be run for all hunks at all `Paths` at once to be able to get the most out of diffs between two trees.
//!
//! ### Associating selected `WorktreeHunks`
//!
//! Selected `WorktreeHunks`, as a subset of all available `WorktreeHunks`, are applied onto `HEAD^{tree}` if applying to `Branches` or to the `WorkspaceCommit` if applying to a `Workspace`.
//! This sets the initial `State` to contain only the selected `WorktreeHunks` and their association to `IntroducingCommits` can be performed as normal.
//!
//! ### Committing `WorktreeHunks`
//!
//! The outcome of associating `WorktreeHunks` with *commits* is the `State` of each `Path` with `WorktreeHunks` for each *commit*. Thus, each *commit* knows how it would look like with
//! all applicable `WorktreeHunks` applied.
//!
//! Commits are effectively amended with the new `State` that contains `WorktreeHunks`, from the commit closest to the `BranchBase` moving upwards to the `BranchTip`, inclusive, which
//! means there is no chance for conflict or unexpected behaviour.
//!
//! Unassociated `WorktreeHunks` either belong to another `Branch` of a `Workspace`, or they would be a candidate to be committed with `BranchTip` as parent.
//!
//! ### Watch out!
//!
//! - Worktree State needs to be converted to what would be Git stage, i.e. has to go through filters first!
//!
//! ### Questions
//!
//! #### What to do with multi-parent commits?
//!
//! In theory, would have to merge the parents, and diff it against the commit. That bears the risk of a conflict (that has been resolved in the commit),
//! so in that case it should be fine to fallback to using the first parent.
mod input;

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use but_core::{TreeChange, UnifiedPatch};
use gix::trace;
pub use input::{InputCommit, InputDiffHunk, InputFile, InputStack};

mod ranges;
pub use ranges::{CalculationError, HunkRange, WorkspaceRanges};
use tracing::instrument;

use crate::ui::HunkLockTarget;

/// Global cache for commit tree diffs.
///
/// Keyed by `commit_id` alone — a commit's parent and tree are embedded in the
/// commit object, so the diff is deterministic for any given commit ID.
/// Commits that are no longer part of the active workspace are pruned
/// automatically in [`new_stacks_to_input_stacks()`].
static COMMIT_DIFF_CACHE: LazyLock<Mutex<HashMap<gix::ObjectId, Vec<InputFile>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn commit_diff_cache() -> std::sync::MutexGuard<'static, HashMap<gix::ObjectId, Vec<InputFile>>> {
    COMMIT_DIFF_CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Types and conversions for use in `tauri`.
pub mod ui;

mod utils;

/// Produce one [`InputStack`] instance for each [`but_graph::workspace::Stack`] for use in [`WorkspaceRanges::try_from_stacks`].
#[instrument(skip_all, err(Debug))]
pub fn new_stacks_to_input_stacks(
    repo: &gix::Repository,
    workspace: &but_graph::Workspace,
) -> anyhow::Result<Vec<InputStack>> {
    let total_commits: usize = workspace
        .stacks
        .iter()
        .map(|s| {
            s.segments
                .iter()
                .map(|seg| seg.commits.len())
                .sum::<usize>()
        })
        .sum();

    tracing::info!(
        num_stacks = workspace.stacks.len(),
        total_commits,
        "processing stacks"
    );

    let mut had_miss = false;

    let result = workspace
        .stacks
        .iter()
        .enumerate()
        .map(|(stack_idx, stack)| {
            let _span = tracing::info_span!(
                "process_stack",
                stack_idx,
                stack_id = ?stack.id,
                num_segments = stack.segments.len(),
            )
            .entered();

            let commits = stack
                .segments
                .iter()
                .flat_map(|s| &s.commits)
                .map(|commit| {
                    let commit_id = commit.id;

                    // Check cache first — commit diffs are immutable (content-addressed).
                    let cached = commit_diff_cache().get(&commit_id).cloned();

                    let files = match cached {
                        Some(cached) => {
                            tracing::debug!(%commit_id, "cache hit");
                            cached
                        }
                        None => {
                            had_miss = true;
                            let commit = repo.find_commit(commit_id)?;
                            let parent_id = commit.parent_ids().next().map(|id| id.detach());
                            let tree_changes = {
                                let _span =
                                    tracing::debug_span!("tree_changes", %commit_id).entered();
                                but_core::diff::tree_changes(repo, parent_id, commit_id)?
                            };
                            let computed = {
                                let _span = tracing::debug_span!(
                                    "tree_changes_to_input_files",
                                    num_changes = tree_changes.len()
                                )
                                .entered();
                                tree_changes_to_input_files(repo, tree_changes)?
                            };
                            commit_diff_cache().insert(commit_id, computed.clone());
                            computed
                        }
                    };

                    Ok(InputCommit { commit_id, files })
                })
                .rev()
                .collect::<anyhow::Result<Vec<_>>>()?;

            Ok(InputStack {
                target: stack
                    .id
                    .map_or(HunkLockTarget::Unidentified, HunkLockTarget::Stack),
                commits_from_base_to_tip: commits,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Evict commits that are no longer part of the workspace.
    {
        let mut cache = commit_diff_cache();
        if had_miss || cache.len() > total_commits {
            let active_commits: HashSet<gix::ObjectId> = workspace
                .stacks
                .iter()
                .flat_map(|s| &s.segments)
                .flat_map(|seg| &seg.commits)
                .map(|c| c.id)
                .collect();
            let before = cache.len();
            cache.retain(|id, _| active_commits.contains(id));
            let evicted = before - cache.len();
            if evicted > 0 {
                tracing::debug!(
                    evicted,
                    remaining = cache.len(),
                    "pruned obsolete commits from commit diff cache"
                );
            }
        }
    }

    Ok(result)
}

/// Turn `changes` with [`TreeChange`] instances into [`InputFile`], one for each input.
pub fn tree_changes_to_input_files(
    repo: &gix::Repository,
    changes: Vec<TreeChange>,
) -> anyhow::Result<Vec<InputFile>> {
    let mut files = Vec::new();
    for change in changes {
        let diff = change.unified_patch(repo, 0)?;
        let Some(UnifiedPatch::Patch { hunks, .. }) = diff else {
            trace::warn!(
                "Skipping change at '{}' as it doesn't have hunks to calculate dependencies for (binary/too large)",
                change.path
            );
            continue;
        };
        let change_type = change.status.kind();
        files.push(InputFile {
            path: change.path,
            hunks: hunks.iter().map(InputDiffHunk::from_unified_diff).collect(),
            change_type,
        })
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_diff_cache_populates_hits_and_prunes_obsolete_commits() -> anyhow::Result<()> {
        let (repo, _tmpdir) = but_testsupport::writable_scenario("independent-commits-multi-stack");
        let ctx = but_ctx::Context::from_repo(repo)?;
        let (_guard, repo, ws, _) = ctx.workspace_and_db()?;

        commit_diff_cache().clear();

        // 1. Cold call populates all commits in the workspace.
        let first = new_stacks_to_input_stacks(&repo, &ws)?;
        let all_commits: HashSet<gix::ObjectId> = ws
            .stacks
            .iter()
            .flat_map(|s| &s.segments)
            .flat_map(|seg| &seg.commits)
            .map(|c| c.id)
            .collect();
        assert!(
            all_commits.len() > 1,
            "expected multiple commits in fixture"
        );
        assert_eq!(commit_diff_cache().len(), all_commits.len());
        for id in &all_commits {
            assert!(commit_diff_cache().contains_key(id));
        }

        // 2. Warm call hits the cache and produces identical InputStacks.
        let second = new_stacks_to_input_stacks(&repo, &ws)?;
        assert_eq!(first, second);
        assert_eq!(commit_diff_cache().len(), all_commits.len());

        // 3. When a stack is removed from the workspace, only its commits are
        //    pruned and the remaining stack's commits stay cached.
        let mut reduced_ws = ws.clone();
        let removed_stack = reduced_ws.stacks.pop().expect("at least two stacks");
        let removed_commits: HashSet<gix::ObjectId> = removed_stack
            .segments
            .iter()
            .flat_map(|seg| &seg.commits)
            .map(|c| c.id)
            .collect();
        let remaining_commits: HashSet<gix::ObjectId> = reduced_ws
            .stacks
            .iter()
            .flat_map(|s| &s.segments)
            .flat_map(|seg| &seg.commits)
            .map(|c| c.id)
            .collect();
        assert!(!removed_commits.is_empty());
        assert!(!remaining_commits.is_empty());

        let reduced = new_stacks_to_input_stacks(&repo, &reduced_ws)?;
        assert_eq!(reduced.len(), reduced_ws.stacks.len());
        assert_eq!(commit_diff_cache().len(), remaining_commits.len());
        for id in &remaining_commits {
            assert!(commit_diff_cache().contains_key(id));
        }
        for id in &removed_commits {
            assert!(!commit_diff_cache().contains_key(id));
        }

        Ok(())
    }
}
