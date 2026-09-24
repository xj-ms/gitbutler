use anyhow::{Context as _, Result};
use but_ctx::{Context, access::RepoExclusive};
use gitbutler_oplog::{
    OplogExt,
    entry::{OperationKind, SnapshotDetails},
};
use gitbutler_reference::RemoteRefname;

use crate::{base, base::BaseBranch};

pub fn set_base_branch(
    ctx: &Context,
    target_branch: &RemoteRefname,
    perm: &mut RepoExclusive,
) -> Result<BaseBranch> {
    let target_ref = gix::refs::FullName::try_from(target_branch.to_string())
        .context("target branch is not a valid reference name")?;
    set_base_branch_ref(ctx, &target_ref, perm)
}

/// Like [`set_base_branch`], but accepts any fully-qualified target reference, which
/// may be a remote-tracking branch (`refs/remotes/*`) or a tag (`refs/tags/*`).
pub fn set_base_branch_ref(
    ctx: &Context,
    target_ref: &gix::refs::FullName,
    perm: &mut RepoExclusive,
) -> Result<BaseBranch> {
    let _ = ctx.create_snapshot(SnapshotDetails::new(OperationKind::SetBaseBranch), perm);
    base::set_base_branch(ctx, perm, target_ref)
}
