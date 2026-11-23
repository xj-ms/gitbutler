use crate::utils::{r, writable_scenario_slow};
use bstr::ByteSlice;
use but_core::ref_metadata::ProjectMeta;
use but_workspace::{
    RefInfo,
    ref_info::Options,
    ui::PushStatus::{CompletelyUnpushed, NothingToPush, UnpushedCommitsRequiringForce},
};

static ASKPASS: std::sync::Once = std::sync::Once::new();

fn fixture(
    name: &str,
) -> anyhow::Result<(
    but_testsupport::gix_testtools::tempfile::TempDir,
    gix::Repository,
    but_meta::VirtualBranchesTomlMetadata,
)> {
    ASKPASS.call_once(but_askpass::disable);
    let (repo, tmp) = writable_scenario_slow(name);
    let remote = tmp.path().join("remote.git");
    let status = std::process::Command::new("git")
        .current_dir(repo.workdir().expect("fixtures have workdirs"))
        .args(["remote", "set-url", "origin"])
        .arg(remote)
        .status()?;
    assert!(
        status.success(),
        "fixture remote URL should be normalized to an absolute path"
    );
    let meta = but_meta::VirtualBranchesTomlMetadata::from_path(
        repo.path().join("virtual-branches.toml"),
    )?;
    Ok((tmp, repo, meta))
}

fn project_meta(repo: &gix::Repository) -> anyhow::Result<ProjectMeta> {
    Ok(ProjectMeta {
        target_ref: Some(but_core::ref_metadata::LocalRefName::new(
            "refs/heads/main".try_into()?,
        )?),
        target_commit_id: Some(repo.rev_parse_single("main")?.detach()),
        push_remote: None,
    })
}

fn head_info(
    repo: &gix::Repository,
    meta: &but_meta::VirtualBranchesTomlMetadata,
) -> anyhow::Result<(RefInfo, but_graph::Workspace)> {
    let mut db = but_testsupport::project_db(repo)?;
    // Adoption already ran, so the fixture worktrees count as active.
    db.worktree_meta_mut().mark_adopted()?;
    but_workspace::head_info_and_workspace(
        repo,
        meta,
        &mut db,
        Options {
            project_meta: project_meta(repo)?,
            traversal: but_graph::init::Options {
                worktrees: true,
                ..Default::default()
            },
            expensive_commit_info: true,
            ..Default::default()
        },
    )
}

fn push(
    repo: &gix::Repository,
    meta: &but_meta::VirtualBranchesTomlMetadata,
    branch: &gix::refs::FullNameRef,
    with_force: bool,
    skip_force_push_protection: bool,
    force_push_protection: bool,
) -> anyhow::Result<gitbutler_git::PushResult> {
    let (info, workspace) = head_info(repo, meta)?;
    let mut db = but_db::DbHandle::new_at_path(":memory:")?;
    but_workspace::legacy::workspace_branch_and_ancestors_push(
        repo,
        &workspace,
        &info,
        &mut db,
        false,
        with_force,
        skip_force_push_protection,
        force_push_protection,
        branch,
        false,
        false,
        Vec::new(),
    )
}

fn apply_remote_tracking_updates(
    repo: &gix::Repository,
    result: &gitbutler_git::PushResult,
) -> anyhow::Result<()> {
    for ((_branch, remote_refname, _remote_branch_name), (_, _, after_sha)) in result
        .branch_to_remote
        .iter()
        .zip(result.branch_sha_updates.iter())
    {
        let status = std::process::Command::new("git")
            .current_dir(repo.workdir().expect("fixtures have workdirs"))
            .args(["update-ref", remote_refname.as_bstr().to_str()?, after_sha])
            .status()?;
        assert!(
            status.success(),
            "git update-ref should update pushed remote-tracking refs"
        );
    }
    Ok(())
}

fn status(info: &RefInfo, branch: &str) -> but_workspace::ui::PushStatus {
    info.lanes()
        .flat_map(|lane| lane.segments)
        .find(|segment| {
            segment
                .ref_info
                .as_ref()
                .is_some_and(|ref_info| ref_info.ref_name.shorten() == branch.as_bytes())
        })
        .unwrap_or_else(|| panic!("fixture should contain branch `{branch}`"))
        .push_status
}

fn logical_scope(info: &RefInfo, branch: &str) -> Vec<String> {
    let branch = gix::refs::Category::LocalBranch
        .to_full_name(branch)
        .expect("valid fixture branch name");
    but_workspace::legacy::push::branch_and_ancestor_segments(info, branch.as_ref())
        .values()
        .map(|segment| {
            segment
                .ref_name()
                .map_or("<anon>".to_string(), |name| name.shorten().to_string())
        })
        .collect()
}

#[test]
fn logical_push_scope_is_selected_branch_plus_ancestors() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push")?;
    let (info, _) = head_info(&repo, &meta)?;

    assert_eq!(logical_scope(&info, "bottom"), ["bottom"]);
    assert_eq!(logical_scope(&info, "middle"), ["middle", "bottom"]);
    assert_eq!(logical_scope(&info, "top"), ["top", "middle", "bottom"]);
    assert_eq!(
        logical_scope(&info, "solo"),
        ["solo"],
        "an unrelated stack must not enter the selected scope"
    );

    Ok(())
}

fn pushed_branches(result: &gitbutler_git::PushResult) -> Vec<&str> {
    result
        .branch_to_remote
        .iter()
        .map(|(branch, _, _)| branch.as_str())
        .collect()
}

#[test]
fn logical_push_scope_crosses_into_the_lane_a_worktree_rests_on() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push-worktree")?;
    let (info, _) = head_info(&repo, &meta)?;

    assert_eq!(
        logical_scope(&info, "wt-on-middle"),
        ["wt-on-middle", "middle", "bottom"],
        "a worktree resting on a stack branch continues into that branch and its ancestors"
    );
    assert_eq!(
        logical_scope(&info, "wt-top"),
        ["wt-top", "wt-bottom"],
        "a worktree based on the target is a stack of its own"
    );
    assert_eq!(
        logical_scope(&info, "top"),
        ["top", "middle", "bottom"],
        "a worktree resting on the stack does not enter the stack's own scope"
    );
    Ok(())
}

#[test]
fn pushing_a_worktree_branch_pushes_the_stack_branches_beneath_it() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push-worktree")?;

    let result = push(
        &repo,
        &meta,
        r("refs/heads/wt-on-middle"),
        false,
        false,
        false,
    )?;
    assert_eq!(
        pushed_branches(&result),
        ["bottom", "middle", "wt-on-middle"],
        "the stack branches beneath the worktree go first, bottom-up"
    );

    apply_remote_tracking_updates(&repo, &result)?;
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(status(&info, "bottom"), NothingToPush);
    assert_eq!(status(&info, "middle"), NothingToPush);
    assert_eq!(status(&info, "wt-on-middle"), NothingToPush);
    assert_eq!(
        status(&info, "top"),
        CompletelyUnpushed,
        "the stack branch above the base is not beneath the worktree"
    );
    assert_eq!(
        status(&info, "wt-bottom"),
        CompletelyUnpushed,
        "an unrelated worktree is untouched"
    );

    let result = push(&repo, &meta, r("refs/heads/wt-top"), false, false, false)?;
    assert_eq!(
        pushed_branches(&result),
        ["wt-bottom", "wt-top"],
        "an independent worktree stack pushes like a workspace stack"
    );
    Ok(())
}

fn assert_refused_and_remote_untouched(
    repo: &gix::Repository,
    meta: &but_meta::VirtualBranchesTomlMetadata,
    branch: &str,
) -> anyhow::Result<()> {
    let err = push(repo, meta, r(branch), false, false, false)
        .expect_err("the workspace commit merges every applied branch");
    assert!(
        err.to_string().contains("GitButler's workspace commit"),
        "{branch}: {err:#}"
    );
    let remote_refs = but_testsupport::git(repo)
        .args([
            "--git-dir",
            "remote.git",
            "for-each-ref",
            "--format=%(refname)",
        ])
        .output()?;
    assert_eq!(
        remote_refs.stdout.as_bstr(),
        "refs/heads/main\n",
        "nothing reached the remote"
    );
    Ok(())
}

#[test]
fn a_worktree_resting_on_the_workspace_commit_is_not_pushed() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push-on-workspace-commit")?;

    assert_refused_and_remote_untouched(&repo, &meta, "refs/heads/wt-on-workspace")?;
    assert_refused_and_remote_untouched(&repo, &meta, "refs/heads/wt-on-it")?;
    Ok(())
}

#[test]
fn a_branch_holding_the_workspace_commit_is_not_pushed() -> anyhow::Result<()> {
    use but_testsupport::CommandExt as _;

    let (_tmp, repo, meta) = fixture("push-on-workspace-commit")?;
    // As an agent's `git checkout -b` on the workspace makes one.
    but_testsupport::git(&repo)
        .args(["checkout", "-q", "-b", "hotfix"])
        .run();
    but_testsupport::git(&repo)
        .args(["commit", "-q", "--allow-empty", "-m", "hotfix"])
        .run();

    assert_refused_and_remote_untouched(&repo, &meta, "refs/heads/hotfix")?;
    Ok(())
}

#[test]
fn pushed_branch_reports_its_name_on_the_remote_it_landed_on() -> anyhow::Result<()> {
    let (tmp, repo, meta) = fixture("push")?;
    // Track `bottom` on a second remote so its own remote differs from the push default,
    // which is derived from the target ref and stays `origin`.
    let fork = tmp.path().join("remote.git");
    let workdir = repo.workdir().expect("fixtures have workdirs");
    // The tracking ref has to exist for the branch to be seen as tracking `fork`, and it
    // points at the base so `bottom` still has commits left to push.
    let base = repo.rev_parse_single("main")?.to_string();
    for args in [
        vec!["remote", "add", "fork", fork.to_str().expect("utf8 path")],
        vec!["config", "branch.bottom.remote", "fork"],
        vec!["config", "branch.bottom.merge", "refs/heads/bottom"],
        vec!["update-ref", "refs/remotes/fork/bottom", base.as_str()],
    ] {
        let status = std::process::Command::new("git")
            .current_dir(workdir)
            .args(args)
            .status()?;
        assert!(status.success(), "fixture setup should succeed");
    }
    // Reopen so the configuration written above is visible.
    let repo = gix::open(repo.path())?;

    let result = push(&repo, &meta, r("refs/heads/bottom"), false, false, false)?;

    assert_eq!(
        result.remote, "origin",
        "the push default still comes from the target ref"
    );
    let (branch, remote_refname, remote_branch_name) = result
        .branch_to_remote
        .first()
        .expect("bottom should have been pushed");
    assert_eq!(branch, "bottom");
    assert_eq!(remote_refname, "refs/remotes/fork/bottom");
    assert_eq!(
        remote_branch_name, "bottom",
        "the branch name on the remote must be stripped of the remote the branch actually landed on, not the push default"
    );

    Ok(())
}

#[test]
fn pushing_bottom_of_stack_reports_only_bottom_as_pushed() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push")?;

    let result = push(&repo, &meta, r("refs/heads/bottom"), false, false, false)?;
    assert_eq!(
        result
            .branch_to_remote
            .iter()
            .map(|(branch, _, _)| branch.as_str())
            .collect::<Vec<_>>(),
        ["bottom"],
        "pushing the bottom branch should not push the top branch"
    );

    apply_remote_tracking_updates(&repo, &result)?;
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(status(&info, "bottom"), NothingToPush);
    assert_eq!(status(&info, "top"), CompletelyUnpushed);

    Ok(())
}

#[test]
fn pushing_top_of_stack_reports_top_as_pushed_after_bottom_is_current() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push")?;

    let bottom_result = push(&repo, &meta, r("refs/heads/bottom"), false, false, false)?;
    apply_remote_tracking_updates(&repo, &bottom_result)?;
    let middle_result = push(&repo, &meta, r("refs/heads/middle"), false, false, false)?;
    apply_remote_tracking_updates(&repo, &middle_result)?;

    let result = push(&repo, &meta, r("refs/heads/top"), false, false, false)?;
    assert_eq!(
        result
            .branch_to_remote
            .iter()
            .map(|(branch, _, _)| branch.as_str())
            .collect::<Vec<_>>(),
        ["top"],
        "once the ancestors are current, pushing the top branch should report only the top"
    );

    apply_remote_tracking_updates(&repo, &result)?;
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(
        logical_scope(&info, "top"),
        ["top", "middle", "bottom"],
        "already-current ancestors remain in the logical synchronization scope"
    );
    assert_eq!(status(&info, "bottom"), NothingToPush);
    assert_eq!(status(&info, "middle"), NothingToPush);
    assert_eq!(status(&info, "top"), NothingToPush);

    Ok(())
}

#[test]
fn pushed_workspace_update_plan_and_execution() -> anyhow::Result<()> {
    let (_tmp, repo, mut meta) = fixture("push")?;
    let (info, workspace) = head_info(&repo, &meta)?;
    let mut db = but_db::DbHandle::new_at_path(":memory:")?;
    let base_oid = repo.find_reference("refs/heads/main")?.peel_to_commit()?.id;
    let project_meta = ProjectMeta {
        target_ref: Some(but_core::ref_metadata::LocalRefName::new(
            "refs/heads/main".try_into()?,
        )?),
        target_commit_id: Some(base_oid),
        push_remote: None,
    };

    // Push workspace and branches to `origin/workspace-sync`.
    but_workspace::legacy::push::workspace_push_all(
        &repo,
        &workspace,
        &project_meta,
        &info,
        &mut db,
        false,
        "workspace-sync",
        false,
        false,
        false,
        false,
        Vec::new(),
    )?;

    // Immediately after pushing, local `gitbutler/workspace` matches `refs/remotes/origin/workspace-sync`.
    assert_eq!(
        but_workspace::legacy::push::pushed_workspace_time_diff_secs(&repo, &project_meta)?,
        None,
        "workspace should not diverge immediately after pushing"
    );
    assert_eq!(
        but_workspace::legacy::push::pushed_workspace_update_plan(&repo, &project_meta)?,
        None
    );

    let pushed_ws_oid = repo
        .find_reference("refs/remotes/origin/workspace-sync")?
        .peel_to_commit()?
        .id;
    let pushed_top_oid = repo.find_reference("refs/heads/top")?.peel_to_commit()?.id;
    let base_oid = repo.find_reference("refs/heads/main")?.peel_to_commit()?.id;

    // Move local `refs/heads/top` and `refs/heads/gitbutler/workspace` back to an older commit (`main`)
    // to simulate the local workspace being behind the pushed upstream workspace.
    but_core::worktree::safe_checkout_from_head(
        base_oid,
        &repo,
        but_core::worktree::checkout::Options {
            skip_head_update: true,
            ..Default::default()
        },
    )?;
    repo.reference(
        "refs/heads/top",
        base_oid,
        gix::refs::transaction::PreviousValue::Any,
        "rewind top for test",
    )?;
    repo.reference(
        "refs/heads/gitbutler/workspace",
        base_oid,
        gix::refs::transaction::PreviousValue::Any,
        "rewind workspace for test",
    )?;

    assert!(
        but_workspace::legacy::push::pushed_workspace_time_diff_secs(&repo, &project_meta)?
            .is_some(),
        "workspace should be detected as diverged when remote tracking ref points to newer managed workspace commit"
    );

    let plan = but_workspace::legacy::push::pushed_workspace_update_plan(&repo, &project_meta)?
        .expect("plan should be available");
    assert_eq!(plan.upstream_ref_name, "refs/remotes/origin/workspace-sync");
    assert_eq!(plan.remote_name, "origin");
    assert_eq!(plan.old_workspace_sha, base_oid.to_string());
    assert_eq!(plan.new_workspace_sha, pushed_ws_oid.to_string());
    assert!(
        !plan.has_uncommitted_changes,
        "expected clean worktree after rewind, got uncommitted files: {:?}",
        plan.uncommitted_files
    );
    assert!(
        plan.branches.iter().any(|b| b.name == "top"
            && b.old_sha == Some(base_oid.to_string())
            && b.new_sha == pushed_top_oid.to_string()),
        "plan should include resetting `top` from base_oid to pushed_top_oid: {:?}",
        plan.branches
    );

    // Staging a file should mark `has_uncommitted_changes = true` and block execution.
    let workdir = repo.workdir().expect("non-bare repo");
    let tracked_file = workdir.join("dirty.txt");
    std::fs::write(&tracked_file, b"dirty content\n")?;
    let add_status = std::process::Command::new("git")
        .current_dir(workdir)
        .args(["add", "dirty.txt"])
        .status()?;
    assert!(add_status.success());
    let dirty_plan =
        but_workspace::legacy::push::pushed_workspace_update_plan(&repo, &project_meta)?
            .expect("plan should still return when dirty");
    assert!(dirty_plan.has_uncommitted_changes);
    assert_eq!(dirty_plan.uncommitted_files, vec!["dirty.txt".to_owned()]);
    assert!(
        but_workspace::legacy::push::execute_pushed_workspace_update(
            &repo,
            &project_meta,
            &mut meta,
        )
        .is_err(),
        "execution must fail when worktree has uncommitted tracked changes"
    );

    // Unstage and remove dirty file, then execute the update.
    let reset_status = std::process::Command::new("git")
        .current_dir(workdir)
        .args(["rm", "-f", "dirty.txt"])
        .status()?;
    assert!(reset_status.success());
    let (executed_plan, updated_meta) =
        but_workspace::legacy::push::execute_pushed_workspace_update(
            &repo,
            &project_meta,
            &mut meta,
        )?;
    assert_eq!(executed_plan.new_workspace_sha, pushed_ws_oid.to_string());
    assert_eq!(updated_meta.push_remote.as_deref(), Some("origin"));
    assert_eq!(
        repo.find_reference("refs/heads/gitbutler/workspace")?
            .peel_to_commit()?
            .id,
        pushed_ws_oid
    );
    assert_eq!(
        repo.find_reference("refs/heads/top")?.peel_to_commit()?.id,
        pushed_top_oid
    );
    assert_eq!(
        but_workspace::legacy::push::pushed_workspace_time_diff_secs(&repo, &updated_meta)?,
        None,
        "workspace should no longer diverge after executing update"
    );

    // Also verify setting up a workspace from a selected branch (`refs/heads/fork-master`)
    // when starting from a fresh / unconfigured `ProjectMeta::default()`.
    repo.reference(
        "refs/heads/fork-master",
        pushed_ws_oid,
        gix::refs::transaction::PreviousValue::Any,
        "create local fork-master at pushed workspace commit",
    )?;
    repo.reference(
        "refs/heads/top",
        base_oid,
        gix::refs::transaction::PreviousValue::Any,
        "rewind top before setup test",
    )?;
    let empty_project_meta = but_core::ref_metadata::ProjectMeta::default();
    let fork_master_ref: gix::refs::FullName = "refs/heads/fork-master".try_into()?;
    let setup_plan = but_workspace::legacy::push::pushed_workspace_setup_plan_for_ref(
        &repo,
        &empty_project_meta,
        fork_master_ref.as_ref(),
        Some("origin"),
    )?
    .expect("setup plan should be returned for a branch pointing to a managed workspace commit");
    assert_eq!(setup_plan.new_workspace_sha, pushed_ws_oid.to_string());
    assert!(setup_plan.branches.iter().any(|b| b.name == "top"));

    let (_executed_setup, setup_meta) =
        but_workspace::legacy::push::execute_pushed_workspace_setup_from_ref(
            &repo,
            &empty_project_meta,
            fork_master_ref.as_ref(),
            Some("origin"),
            &mut meta,
        )?;
    assert_eq!(setup_meta.push_remote.as_deref(), Some("origin"));
    assert_eq!(
        but_workspace::legacy::push::workspace_remote_branch(&repo)?.as_deref(),
        Some("fork-master")
    );
    assert_eq!(
        repo.find_reference("refs/heads/top")?.peel_to_commit()?.id,
        pushed_top_oid
    );
    assert_eq!(
        repo.head()?
            .referent_name()
            .map(|n| n.as_bstr().to_string()),
        Some("refs/heads/gitbutler/workspace".to_owned())
    );

    // 7. Submodule update: when the pushed workspace commit modifies a submodule gitlink (`160000`),
    // `pushed_workspace_update_plan` lists it in `plan.submodules` and `execute_pushed_workspace_update`
    // updates the submodule worktree to the new gitlink SHA.
    let sub_remote_dir = _tmp.path().join("sub-update-remote.git");
    assert!(
        std::process::Command::new("git")
            .args(["init", "--bare", sub_remote_dir.to_str().unwrap()])
            .status()?
            .success()
    );
    let sub_workdir = workdir.join("sub");
    assert!(
        std::process::Command::new("git")
            .args(["init", sub_workdir.to_str().unwrap()])
            .status()?
            .success()
    );
    for args in [
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.com"],
        vec!["checkout", "-b", "main"],
        vec![
            "remote",
            "add",
            "origin",
            sub_remote_dir.to_str().unwrap(),
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&sub_workdir)
                .args(&args)
                .status()?
                .success()
        );
    }
    std::fs::write(sub_workdir.join("sub.txt"), b"v1\n")?;
    for args in [
        vec!["add", "sub.txt"],
        vec!["commit", "-m", "sub v1"],
        vec!["push", "origin", "main"],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&sub_workdir)
                .args(&args)
                .status()?
                .success()
        );
    }
    let sub_v1_oid = gix::open(&sub_workdir)?.head_id()?.detach();

    std::fs::write(sub_workdir.join("sub.txt"), b"v2\n")?;
    for args in [
        vec!["add", "sub.txt"],
        vec!["commit", "-m", "sub v2"],
        vec!["push", "origin", "main"],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(&sub_workdir)
                .args(&args)
                .status()?
                .success()
        );
    }
    let sub_v2_oid = gix::open(&sub_workdir)?.head_id()?.detach();

    std::fs::write(
        workdir.join(".gitmodules"),
        format!(
            "[submodule \"sub\"]\n\tpath = sub\n\turl = {}\n",
            sub_remote_dir.display().to_string().replace('\\', "/")
        ),
    )?;
    // Record sub v1 on the local workspace commit first so `sub` is already a tracked submodule
    for args in [
        vec![
            "config",
            "submodule.sub.url",
            sub_remote_dir.to_str().unwrap(),
        ],
        vec![
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{sub_v1_oid},sub"),
        ],
        vec!["add", ".gitmodules"],
        vec!["commit", "--amend", "--no-edit"],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(workdir)
                .args(&args)
                .status()?
                .success()
        );
    }
    let repo = gix::open(repo.path())?;
    let ws_with_sub_v1_oid = repo.head_id()?.detach();

    // Advance the upstream workspace commit to sub v2 (keeping the same managed parents)
    for args in [
        vec![
            "update-index",
            "--cacheinfo",
            &format!("160000,{sub_v2_oid},sub"),
        ],
        vec!["commit", "--amend", "--no-edit"],
    ] {
        assert!(
            std::process::Command::new("git")
                .current_dir(workdir)
                .args(&args)
                .status()?
                .success()
        );
    }
    let repo = gix::open(repo.path())?;
    let ws_with_sub_oid = repo.head_id()?.detach();
    repo.reference(
        "refs/remotes/origin/fork-master",
        ws_with_sub_oid,
        gix::refs::transaction::PreviousValue::Any,
        "advance remote workspace commit with submodule",
    )?;
    // Rewind local workspace to ws_with_sub_v1_oid and rewind submodule worktree to sub_v1_oid
    assert!(
        std::process::Command::new("git")
            .current_dir(workdir)
            .args(["reset", "--hard", &ws_with_sub_v1_oid.to_string()])
            .status()?
            .success()
    );
    assert!(
        std::process::Command::new("git")
            .current_dir(&sub_workdir)
            .args(["checkout", "--detach", &sub_v1_oid.to_string()])
            .status()?
            .success()
    );

    let repo = gix::open(repo.path())?;
    let sub_plan =
        but_workspace::legacy::push::pushed_workspace_update_plan(&repo, &setup_meta)?
            .expect("plan should be eligible when upstream workspace commit has submodule update");
    assert_eq!(
        sub_plan.submodules,
        vec![but_workspace::legacy::push::PushedWorkspaceBranchUpdate {
            name: "sub".to_owned(),
            old_sha: Some(sub_v1_oid.to_string()),
            new_sha: sub_v2_oid.to_string(),
        }]
    );

    but_workspace::legacy::push::execute_pushed_workspace_update(&repo, &setup_meta, &mut meta)?;
    let updated_sub_oid = gix::open(&sub_workdir)?.head_id()?.detach();
    assert_eq!(
        updated_sub_oid, sub_v2_oid,
        "submodule worktree should be updated to sub_v2_oid after execute_pushed_workspace_update"
    );

    Ok(())
}

#[test]
fn force_push_protection_is_observed_when_pushing_bottom_branch() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push-requiring-force")?;
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(status(&info, "bottom"), UnpushedCommitsRequiringForce);

    let err = push(&repo, &meta, r("refs/heads/bottom"), true, false, true)
        .expect_err("force-with-lease should reject the stale remote branch");
    let err = format!("{err:#}");
    assert!(
        err.contains("force push was blocked")
            && err.contains("--force-with-lease")
            && err.contains("--force-if-includes"),
        "error should come from force push protection: {err:#}"
    );

    let result = push(&repo, &meta, r("refs/heads/bottom"), true, true, true)?;
    assert_eq!(
        result
            .branch_to_remote
            .iter()
            .map(|(branch, _, _)| branch.as_str())
            .collect::<Vec<_>>(),
        ["bottom"],
        "skipping force push protection should allow pushing the rewritten bottom branch"
    );

    Ok(())
}

#[test]
fn force_push_protection_is_observed_when_pushing_top_branch() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push-requiring-force")?;

    let err = push(&repo, &meta, r("refs/heads/top"), true, false, true)
        .expect_err("pushing the top branch should observe bottom branch force protection first");
    let err = format!("{err:#}");
    assert!(
        err.contains("force push was blocked")
            && err.contains("--force-with-lease")
            && err.contains("--force-if-includes"),
        "error should come from force push protection: {err:#}"
    );

    let result = push(&repo, &meta, r("refs/heads/top"), true, true, true)?;
    assert_eq!(
        result
            .branch_to_remote
            .iter()
            .map(|(branch, _, _)| branch.as_str())
            .collect::<Vec<_>>(),
        ["bottom", "top"],
        "skipping force push protection should allow pushing the bottom ancestor and top branch"
    );

    Ok(())
}

#[test]
fn pushing_with_an_ordinary_branch_checked_out_pushes_it_and_its_ancestors() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push-single-branch")?;
    assert!(
        repo.find_reference("refs/heads/gitbutler/workspace")
            .is_err(),
        "the fixture has no workspace branch"
    );
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(
        logical_scope(&info, "top"),
        ["top", "bottom"],
        "the checked-out branch and its ancestor form the push scope"
    );
    assert_eq!(
        status(&info, "bottom"),
        CompletelyUnpushed,
        "nothing has been pushed yet"
    );
    assert_eq!(
        status(&info, "top"),
        CompletelyUnpushed,
        "nothing has been pushed yet"
    );

    let result = push(&repo, &meta, r("refs/heads/top"), false, false, false)?;
    assert_eq!(
        result
            .branch_to_remote
            .iter()
            .map(|(branch, _, _)| branch.as_str())
            .collect::<Vec<_>>(),
        ["bottom", "top"],
        "the checked-out branch and its unpushed ancestor are pushed"
    );

    apply_remote_tracking_updates(&repo, &result)?;
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(
        status(&info, "bottom"),
        NothingToPush,
        "the ancestor is current after the push"
    );
    assert_eq!(
        status(&info, "top"),
        NothingToPush,
        "the checked-out branch is current after the push"
    );
    assert_eq!(
        repo.head_name()?.map(|name| name.as_bstr().to_string()),
        Some("refs/heads/top".into()),
        "pushing leaves the checkout untouched"
    );
    Ok(())
}
