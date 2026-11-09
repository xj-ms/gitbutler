use but_core::RefMetadata;
use but_workspace::branch::apply::OutcomeStatus;
use gitbutler_oplog::{OplogExt as _, RestoreKind};

#[test]
fn apply_only_threads_returned_workspace_back_into_context_cache() -> anyhow::Result<()> {
    let (repo, _tmp) = crate::support::writable_scenario("checkout-head-info");
    crate::support::persist_default_target(&repo)?;

    let mut ctx = but_ctx::Context::from_repo_for_testing(repo)?.with_memory_app_cache();
    let feature = gix::refs::FullName::try_from("refs/heads/feature")?;
    let sibling = gix::refs::FullName::try_from("refs/heads/sibling")?;

    let first = but_api::branch::apply_only(&mut ctx, feature.as_ref())?;
    assert!(
        first.applied_branches.contains(&feature),
        "first apply should persist the requested feature branch: {:?}",
        first.applied_branches
    );

    let second = but_api::branch::apply_only(&mut ctx, sibling.as_ref())?;
    assert!(
        second.applied_branches.contains(&sibling),
        "second apply should use the cached workspace updated by the first apply: {:?}",
        second.applied_branches
    );

    let workspace = crate::support::workspace_graph(&ctx)?;
    assert!(
        workspace.contains("feature"),
        "cached workspace should still contain the first applied branch after the second apply:\n{workspace}"
    );
    assert!(
        workspace.contains("sibling"),
        "cached workspace should contain the second applied branch:\n{workspace}"
    );

    Ok(())
}

#[test]
fn undo_apply_from_ad_hoc_checkout_restores_checkout_and_workspace() -> anyhow::Result<()> {
    let (repo, _tmp) = crate::support::writable_scenario("checkout-head-info");
    crate::support::persist_default_target(&repo)?;
    let mut ctx = but_ctx::Context::from_repo_for_testing(repo)?.with_memory_app_cache();
    let feature = gix::refs::FullName::try_from("refs/heads/feature")?;
    let sibling = gix::refs::FullName::try_from("refs/heads/sibling")?;

    but_api::branch::apply_only(&mut ctx, feature.as_ref())?;
    let workspace_before = ctx
        .repo
        .get()?
        .find_reference(but_core::WORKSPACE_REF_NAME)?
        .peel_to_id()?
        .detach();
    but_api::branch::branch_checkout(&mut ctx, sibling.clone())?;
    let checkout_before = ctx.repo.get()?.head_id()?.detach();

    let outcome = but_api::branch::apply(&mut ctx, feature.as_ref())?;
    assert!(
        outcome.status.persisted_mutation(),
        "applying from the ad-hoc checkout must mutate the managed workspace"
    );
    assert_eq!(
        ctx.repo
            .get()?
            .head_name()?
            .expect("apply leaves HEAD symbolic")
            .as_bstr(),
        but_core::WORKSPACE_REF_NAME,
        "applying from an ad-hoc checkout must enter the managed workspace"
    );

    let snapshot = ctx
        .snapshots_iter(None, Vec::new(), None)?
        .next()
        .expect("a successful apply records its pre-apply snapshot")?;
    let mut guard = ctx.exclusive_worktree_access();
    ctx.restore_snapshot(
        snapshot.commit_id,
        RestoreKind::RestoreFromSnapshotViaUndo,
        guard.write_permission(),
    )?;
    drop(guard);

    assert_eq!(
        ctx.repo
            .get()?
            .head_name()?
            .expect("restored HEAD is symbolic"),
        sibling,
        "undoing the apply must return to the original ad-hoc branch"
    );
    assert_eq!(
        ctx.repo.get()?.head_id()?,
        checkout_before,
        "undoing the apply must restore the original ad-hoc commit"
    );
    assert_eq!(
        ctx.repo
            .get()?
            .find_reference(but_core::WORKSPACE_REF_NAME)?
            .peel_to_id()?,
        workspace_before,
        "undoing the apply must restore the prior managed workspace commit"
    );

    Ok(())
}

fn apply(
    ctx: &mut but_ctx::Context,
    branch: &str,
    approved: &[&str],
) -> anyhow::Result<but_workspace::branch::apply::Outcome> {
    let branch = gix::refs::FullName::try_from(format!("refs/heads/{branch}"))?;
    but_api::branch::apply_with_confirmation(
        ctx,
        branch.as_ref(),
        Some(42),
        approved
            .iter()
            .map(|name| format!("refs/heads/{name}"))
            .collect(),
    )
}

fn references(
    ctx: &but_ctx::Context,
) -> anyhow::Result<std::collections::BTreeMap<gix::refs::FullName, gix::refs::Target>> {
    ctx.repo
        .get()?
        .references()?
        .all()?
        .map(|reference| {
            let reference = reference.map_err(anyhow::Error::from_boxed)?;
            Ok((reference.inner.name, reference.inner.target))
        })
        .collect()
}

#[test]
fn apply_preserves_existing_behavior_without_stacks_to_unapply() -> anyhow::Result<()> {
    let (repo, _tmp) = crate::support::writable_scenario("apply-conflicts");
    crate::support::persist_default_target(&repo)?;
    let mut ctx = but_ctx::Context::from_repo_for_testing(repo)?.with_memory_app_cache();
    apply(&mut ctx, "feature", &[])?;
    apply(&mut ctx, "clean", &[])?;

    // Keep the feature commit as an anonymous workspace parent, rather than
    // a named stack that the user could approve for unapplying.
    {
        let mut guard = ctx.exclusive_worktree_access();
        let _perm = guard.write_permission();
        let repo = ctx.repo.get()?;
        repo.find_reference("refs/heads/feature")?.delete()?;
        let mut meta = ctx.meta()?;
        let workspace_ref = gix::refs::FullName::try_from("refs/heads/gitbutler/workspace")?;
        let mut workspace = meta.workspace(workspace_ref.as_ref())?;
        workspace.remove_segment("refs/heads/feature".try_into()?);
        meta.set_workspace(&workspace)?;
    }
    ctx.invalidate_workspace_cache()?;

    let out = apply(&mut ctx, "incoming", &[])?;
    assert_eq!(
        out.status,
        OutcomeStatus::Applied,
        "conflicts that do not unapply named stacks must retain the original apply behavior"
    );
    assert!(
        out.conflicting_stacks.is_empty(),
        "no named stacks should require confirmation"
    );
    assert!(
        out.workspace_merge
            .as_ref()
            .is_some_and(|merge| merge.has_conflicts()),
        "the fixture must exercise a real merge conflict, not a clean apply"
    );
    let repo = ctx.repo.get()?;
    assert_eq!(
        std::fs::read_to_string(
            repo.workdir()
                .expect("fixture has a worktree")
                .join("a.txt")
        )?,
        "incoming\n",
        "the incoming branch must actually be checked out without confirmation"
    );
    assert!(
        crate::support::fresh_head_info(&ctx)?
            .stacks
            .iter()
            .flat_map(|stack| &stack.segments)
            .any(|segment| segment
                .ref_info
                .as_ref()
                .is_some_and(|info| info.ref_name.as_bstr() == "refs/heads/clean")),
        "existing compatible stacks must remain applied"
    );
    Ok(())
}

#[test]
fn apply_conflicts_require_confirmation_and_recheck_affected_stacks() -> anyhow::Result<()> {
    let (repo, _tmp) = crate::support::writable_scenario("apply-conflicts");
    crate::support::persist_default_target(&repo)?;
    let workdir = repo
        .workdir()
        .expect("fixture has a working tree")
        .to_owned();
    let mut ctx = but_ctx::Context::from_repo_for_testing(repo)?.with_memory_app_cache();
    assert_eq!(
        apply(&mut ctx, "feature", &[])?.status,
        OutcomeStatus::Applied,
        "a clean apply does not need confirmation"
    );
    apply(&mut ctx, "clean", &[])?;
    std::fs::write(workdir.join("clean.txt"), "uncommitted\n")?;

    let workspace_ref = gix::refs::FullName::try_from("refs/heads/gitbutler/workspace")?;
    let refs_before = references(&ctx)?;
    let metadata_before = ctx.meta()?.workspace(workspace_ref.as_ref())?.clone();
    let index_before = std::fs::read(workdir.join(".git").join("index"))?;
    let out = apply(&mut ctx, "incoming", &[])?;
    assert_eq!(
        out.status,
        OutcomeStatus::ConflictAborted,
        "conflicting stacks must stay applied until the user confirms"
    );
    assert_eq!(
        out.conflicting_stacks.len(),
        1,
        "only the feature stack conflicts with the incoming branch"
    );
    let conflict = &out.conflicting_stacks[0];
    assert_eq!(
        conflict.ref_name.as_bstr(),
        "refs/heads/feature",
        "the confirmation identifies the stack that would be unapplied"
    );
    assert_eq!(
        conflict.paths,
        [bstr::BString::from("a.txt"), bstr::BString::from("b.txt")],
        "report every conflicting file, not only the first fail-fast merge conflict"
    );
    assert_eq!(
        references(&ctx)?,
        refs_before,
        "cancelling must leave all branch refs unchanged"
    );
    assert_eq!(
        ctx.meta()?.workspace(workspace_ref.as_ref())?.clone(),
        metadata_before,
        "cancelling must leave workspace metadata unchanged"
    );
    assert_eq!(
        std::fs::read(workdir.join(".git").join("index"))?,
        index_before,
        "cancelling must leave the index unchanged"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join("a.txt"))?,
        "feature\n",
        "cancelling must leave the worktree unchanged"
    );

    apply(&mut ctx, "other", &[])?;
    let refs_before = references(&ctx)?;
    let out = apply(&mut ctx, "incoming", &["feature"])?;
    assert_eq!(
        out.status,
        OutcomeStatus::ConflictAborted,
        "stale confirmation must not unapply an additional stack"
    );
    assert_eq!(
        out.conflicting_stacks.len(),
        2,
        "the updated confirmation includes the newly conflicting stack"
    );
    assert_eq!(
        references(&ctx)?,
        refs_before,
        "stale confirmation must leave the workspace unchanged"
    );

    let out = apply(&mut ctx, "incoming", &["feature", "other"])?;
    assert_eq!(
        out.status,
        OutcomeStatus::Applied,
        "explicitly approving all conflicting stacks allows the apply"
    );
    let head_info = crate::support::fresh_head_info(&ctx)?;
    let applied_names: Vec<_> = head_info
        .stacks
        .iter()
        .flat_map(|stack| &stack.segments)
        .filter_map(|segment| {
            segment
                .ref_info
                .as_ref()
                .map(|info| info.ref_name.to_string())
        })
        .collect();
    assert!(
        applied_names.contains(&"refs/heads/incoming".into())
            && applied_names.contains(&"refs/heads/clean".into())
            && !applied_names.contains(&"refs/heads/feature".into())
            && !applied_names.contains(&"refs/heads/other".into()),
        "apply retains the clean stack and only unapplies the approved conflicting stacks"
    );
    let repo = ctx.repo.get()?;
    assert!(
        repo.try_find_reference("refs/heads/feature")?.is_some()
            && repo.try_find_reference("refs/heads/other")?.is_some(),
        "unapplying must preserve both conflicting branches"
    );
    let incoming = gix::refs::FullName::try_from("refs/heads/incoming")?;
    assert_eq!(
        ctx.meta()?.branch(incoming.as_ref())?.review.pull_request,
        Some(42),
        "confirmed apply must retain the pull request association"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join("a.txt"))?,
        "incoming\n",
        "confirmation must check out the incoming branch's changes"
    );
    assert_eq!(
        std::fs::read_to_string(workdir.join("clean.txt"))?,
        "uncommitted\n",
        "applying after confirmation must preserve unrelated uncommitted changes"
    );
    Ok(())
}
