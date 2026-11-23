use anyhow::anyhow;
use but_core::TreeStatusKind;
use gix::bstr::{BString, ByteSlice};

use crate::ui::HunkLockTarget;

/// A whole stack for the purpose of generating hunk locking information from it, for use in [`WorkspaceRanges::try_from_stacks()`](crate::WorkspaceRanges::try_from_stacks()) .
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputStack {
    /// The stack that contains [commits](InputStack::commits_from_base_to_tip).
    pub target: HunkLockTarget,
    /// The commits in the stack.
    ///
    /// **The commits are ordered from the base to the tip of the stack (application order)**.
    ///
    /// TODO: Why? This is inconsistent with the rest of the codebase.
    pub commits_from_base_to_tip: Vec<InputCommit>,
}

/// A commit along with the files that it changes, used in [`InputStack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputCommit {
    /// The id of the commit this instance refers to.
    pub commit_id: gix::ObjectId,
    /// The files were changed by this commit.
    pub files: Vec<InputFile>,
}

/// A single file changed in an [`InputCommit`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFile {
    /// The worktree-relative path to the file.
    pub path: BString,
    /// The hunks that changed in this file.
    pub hunks: Vec<InputDiffHunk>,
    /// The kind of change of the parent file.
    pub change_type: TreeStatusKind,
}

/// A
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputDiffHunk {
    /// The 1-based line number at which the previous version of the file started.
    pub old_start: u32,
    /// The non-zero amount of lines included in the previous version of the file.
    pub old_lines: u32,
    /// The 1-based line number at which the new version of the file started.
    pub new_start: u32,
    /// The non-zero amount of lines included in the new version of the file.
    pub new_lines: u32,
    /// Optional explicit `[gix-merge-insert-order: <i32>]` directive order for this hunk.
    pub insert_order: Option<i32>,
}

impl InputDiffHunk {
    /// Compute the amount of lines that are left when subtracting old-lines from new-lines.
    pub fn net_lines(&self) -> anyhow::Result<i32> {
        // TODO: use `checked_signed_diff` instead when stable.
        (self.new_lines as i64)
            .checked_sub(self.old_lines as i64)
            .and_then(|n| i32::try_from(n).ok())
            .ok_or(anyhow!("u32 -> i32 conversion overflow"))
    }
}

impl InputDiffHunk {
    /// Create a new instance from unified `diff`.
    pub fn from_unified_diff(
        but_core::unified_diff::DiffHunk {
            old_start,
            old_lines,
            new_start,
            new_lines,
            diff,
        }: &but_core::unified_diff::DiffHunk,
    ) -> Self {
        let segments = ordered_segments(&added_lines(diff));
        InputDiffHunk {
            old_start: *old_start,
            old_lines: *old_lines,
            new_start: *new_start,
            new_lines: *new_lines,
            insert_order: segments.first().and_then(|(_, order)| *order),
        }
    }

    /// Create one or more instances from unified `diff`, splitting pure-insertion
    /// hunks (`old_lines == 0`) at `[gix-merge-insert-order: <i32>]` segment boundaries.
    pub fn split_from_unified_diff(hunk: &but_core::unified_diff::DiffHunk) -> Vec<Self> {
        if hunk.old_lines == 0 && hunk.new_lines > 0 {
            let added = added_lines(&hunk.diff);
            if added.len() == hunk.new_lines as usize {
                let segments = ordered_segments(&added);
                if segments.len() > 1 {
                    return segments
                        .into_iter()
                        .map(|(range, insert_order)| InputDiffHunk {
                            old_start: hunk.old_start,
                            old_lines: 0,
                            new_start: hunk.new_start + range.start as u32,
                            new_lines: (range.end - range.start) as u32,
                            insert_order,
                        })
                        .collect();
                }
            }
        }
        vec![Self::from_unified_diff(hunk)]
    }
}

pub(crate) fn added_lines(diff: &BString) -> Vec<&[u8]> {
    diff.lines()
        .skip(1)
        .filter(|line| line.starts_with(b"+"))
        .map(|line| &line[1..])
        .collect()
}

pub(crate) fn ordered_segments(
    added_lines: &[&[u8]],
) -> Vec<(std::ops::Range<usize>, Option<i32>)> {
    gix::merge::blob::builtin_driver::text::split_ordered_line_ranges(added_lines.len(), |i| {
        added_lines[i]
    })
}
