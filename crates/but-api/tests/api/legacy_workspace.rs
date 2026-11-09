#[test]
fn head_info_discovers_worktrees_through_the_database() -> anyhow::Result<()> {
    let (repo, _tmp) = crate::support::repo_with_feature_branch()?;
    let mut ctx = but_ctx::Context::from_repo_for_testing(repo)?.with_memory_app_cache();
    ctx.settings.feature_flags.worktree_manipulation = true;

    assert!(
        !ctx.db.get_cache()?.worktree_meta().adoption_ran()?,
        "worktree adoption has not run before the workspace graph is built"
    );
    but_api::legacy::workspace::head_info(&ctx)?;
    assert!(
        ctx.db.get_cache()?.worktree_meta().adoption_ran()?,
        "head-info construction passes the feature-gated database handle into graph discovery"
    );
    Ok(())
}

#[test]
fn update_stack_order_reconciles_stale_metadata_and_persists_across_head_info() -> anyhow::Result<()>
{
    let (repo, _tmp) = crate::support::writable_scenario("checkout-head-info");
    crate::support::persist_default_target(&repo)?;

    let mut ctx = but_ctx::Context::from_repo_for_testing(repo)?.with_memory_app_cache();
    let feature = gix::refs::FullName::try_from("refs/heads/feature")?;
    let sibling = gix::refs::FullName::try_from("refs/heads/sibling")?;
    let top = gix::refs::FullName::try_from("refs/heads/top")?;

    but_api::branch::apply_only(&mut ctx, feature.as_ref())?;
    but_api::branch::apply_only(&mut ctx, sibling.as_ref())?;
    crate::support::create_empty_branch_above(&mut ctx, &top, &feature)?;

    // Move `feature` above `top` so `feature` is the new top segment of the stack while
    // `virtual_branches.toml` was not manually rewritten by `move_branch`.
    but_api::branch::move_branch(
        &mut ctx,
        feature.as_ref(),
        top.as_ref(),
        but_core::DryRun::No,
    )?;

    let initial_info = but_api::legacy::workspace::head_info(&ctx)?;
    let initial_ids: Vec<_> = initial_info
        .stacks
        .iter()
        .map(|s| s.id.expect("managed workspace stacks have IDs"))
        .collect();
    assert_eq!(
        initial_ids.len(),
        2,
        "workspace should have two applied stacks"
    );

    let reordered_ids: Vec<_> = initial_ids.iter().copied().rev().collect();
    let updates = reordered_ids
        .iter()
        .enumerate()
        .map(
            |(order, &id)| gitbutler_branch::BranchUpdateRequest {
                id: Some(id),
                order: Some(order),
            },
        )
        .collect();

    but_api::legacy::virtual_branches::update_stack_order(&mut ctx, updates)?;

    let after_info = but_api::legacy::workspace::head_info(&ctx)?;
    let after_ids: Vec<_> = after_info
        .stacks
        .iter()
        .map(|s| s.id.expect("managed workspace stacks have IDs"))
        .collect();
    assert_eq!(
        after_ids, reordered_ids,
        "head_info must reflect the reordered stacks even after top-branch moves"
    );

    Ok(())
}

