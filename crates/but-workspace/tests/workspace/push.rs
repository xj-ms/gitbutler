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
        target_ref: Some("refs/remotes/origin/main".try_into()?),
        target_commit_id: Some(repo.rev_parse_single("main")?.detach()),
        push_remote: None,
    })
}

fn head_info(
    repo: &gix::Repository,
    meta: &but_meta::VirtualBranchesTomlMetadata,
) -> anyhow::Result<(RefInfo, but_graph::Workspace)> {
    but_workspace::head_info_and_workspace(
        repo,
        meta,
        &mut but_testsupport::project_db(repo)?,
        Options {
            project_meta: project_meta(repo)?,
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
        &project_meta(repo)?,
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
    info.stacks
        .iter()
        .flat_map(|stack| &stack.segments)
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
        .filter_map(|segment| {
            segment
                .ref_info
                .as_ref()
                .map(|ref_info| ref_info.ref_name.shorten().to_string())
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
fn workspace_push_all_persists_tracking_branch_and_pushes_workspace_plus_out_of_sync_branches()
-> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push")?;

    // First push `bottom` so it is already up to date on the remote, leaving `middle`, `top`, and `solo` out of sync.
    let bottom_result = push(&repo, &meta, r("refs/heads/bottom"), false, false, false)?;
    apply_remote_tracking_updates(&repo, &bottom_result)?;

    let (info, workspace) = head_info(&repo, &meta)?;
    assert_eq!(status(&info, "bottom"), NothingToPush);
    assert_eq!(status(&info, "middle"), CompletelyUnpushed);
    assert_eq!(status(&info, "top"), CompletelyUnpushed);
    assert_eq!(status(&info, "solo"), CompletelyUnpushed);

    let mut db = but_db::DbHandle::new_at_path(":memory:")?;
    let result = but_workspace::legacy::push::workspace_push_all(
        &repo,
        &workspace,
        &project_meta(&repo)?,
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

    assert_eq!(result.remote, "origin");
    assert_eq!(
        result
            .branch_to_remote
            .iter()
            .map(|(branch, remote_ref, remote_branch)| (
                branch.as_str(),
                remote_ref.to_string(),
                remote_branch.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "gitbutler/workspace",
                "refs/remotes/origin/workspace-sync".to_string(),
                "workspace-sync"
            ),
            ("solo", "refs/remotes/origin/solo".to_string(), "solo"),
            ("middle", "refs/remotes/origin/middle".to_string(), "middle"),
            ("top", "refs/remotes/origin/top".to_string(), "top"),
        ]
    );

    // Reopen repo to verify persisted config and ensure `head_info` still succeeds with workspace tracking configured.
    let repo = gix::open(repo.path())?;
    assert_eq!(
        but_workspace::legacy::push::workspace_remote_branch(&repo)?.as_deref(),
        Some("workspace-sync")
    );

    apply_remote_tracking_updates(&repo, &result)?;
    let (info, _) = head_info(&repo, &meta)?;
    assert_eq!(status(&info, "bottom"), NothingToPush);
    assert_eq!(status(&info, "middle"), NothingToPush);
    assert_eq!(status(&info, "top"), NothingToPush);
    assert_eq!(status(&info, "solo"), NothingToPush);

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
fn push_works_when_target_ref_is_a_tag() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push")?;
    let base_id = repo.rev_parse_single("main")?.detach();
    repo.reference(
        "refs/tags/v1.0.0",
        base_id,
        gix::refs::transaction::PreviousValue::Any,
        "test",
    )?;
    let tag_project_meta = ProjectMeta {
        target_ref: Some("refs/tags/v1.0.0".try_into()?),
        target_commit_id: Some(base_id),
        push_remote: None,
    };

    let (info, workspace) = but_workspace::head_info_and_workspace(
        &repo,
        &meta,
        &mut but_testsupport::project_db(&repo)?,
        Options {
            project_meta: tag_project_meta.clone(),
            expensive_commit_info: true,
            ..Default::default()
        },
    )?;
    assert_eq!(status(&info, "solo"), CompletelyUnpushed);

    let mut db = but_db::DbHandle::new_at_path(":memory:")?;
    let result = but_workspace::legacy::workspace_branch_and_ancestors_push(
        &repo,
        &workspace,
        &tag_project_meta,
        &info,
        &mut db,
        false,
        false,
        false,
        false,
        r("refs/heads/solo"),
        false,
        false,
        Vec::new(),
    )?;
    assert_eq!(result.remote, "origin");
    apply_remote_tracking_updates(&repo, &result)?;

    let (info_after, _) = but_workspace::head_info_and_workspace(
        &repo,
        &meta,
        &mut but_testsupport::project_db(&repo)?,
        Options {
            project_meta: tag_project_meta,
            expensive_commit_info: true,
            ..Default::default()
        },
    )?;
    assert_eq!(
        status(&info_after, "solo"),
        NothingToPush,
        "remote-tracking branch should be deduced from default remote even when target_ref is a tag"
    );

    Ok(())
}

