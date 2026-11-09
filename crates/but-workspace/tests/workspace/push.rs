use bstr::ByteSlice;
use but_core::ref_metadata::ProjectMeta;
use but_workspace::{
    RefInfo,
    ref_info::Options,
    ui::PushStatus::{CompletelyUnpushed, NothingToPush, UnpushedCommitsRequiringForce},
};

use crate::utils::{r, writable_scenario_slow};

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
fn workspace_push_all_pushes_out_of_sync_tag_and_branch_bases() -> anyhow::Result<()> {
    let (_tmp, repo, meta) = fixture("push")?;
    let base_id = repo.rev_parse_single("main")?.detach();
    repo.reference(
        "refs/tags/v1.0.0-base",
        base_id,
        gix::refs::transaction::PreviousValue::Any,
        "test",
    )?;
    let tag_project_meta = ProjectMeta {
        target_ref: Some(but_core::ref_metadata::LocalRefName::new(
            "refs/tags/v1.0.0-base".try_into()?,
        )?),
        target_commit_id: Some(base_id),
        push_remote: None,
    };

    // Initially the tag does not exist on origin, so `check_out_of_sync_base_target` reports it.
    let out_of_sync_tag = but_workspace::legacy::push::check_out_of_sync_base_target(
        &repo,
        &tag_project_meta,
        "origin",
    )?;
    assert_eq!(
        out_of_sync_tag,
        Some(but_workspace::legacy::push::OutOfSyncBaseTarget {
            path: None,
            ref_name: "refs/tags/v1.0.0-base".to_owned(),
            short_name: "v1.0.0-base".to_owned(),
            remote_name: "origin".to_owned(),
            is_tag: true,
            target_sha: base_id.to_string(),
            remote_sha: None,
        })
    );

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

    let mut db = but_db::DbHandle::new_at_path(":memory:")?;
    let result = but_workspace::legacy::push::workspace_push_all(
        &repo,
        &workspace,
        &tag_project_meta,
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

    assert!(
        result
            .branch_to_remote
            .iter()
            .any(|(local, remote_ref, short)| {
                local == "refs/tags/v1.0.0-base"
                    && remote_ref.as_bstr() == "refs/tags/v1.0.0-base"
                    && short == "v1.0.0-base"
            }),
        "out-of-sync tag base should be pushed with workspace_push_all"
    );

    // After pushing, the tag now exists on origin and is in sync.
    assert_eq!(
        but_workspace::legacy::push::check_out_of_sync_base_target(
            &repo,
            &tag_project_meta,
            "origin"
        )?,
        None,
        "tag base should be synced on remote after push_all"
    );

    // Now test a branch base whose remote-tracking ref is behind `target_commit_id`.
    let solo_id = repo.rev_parse_single("solo")?.detach();
    let branch_project_meta = ProjectMeta {
        target_ref: Some(but_core::ref_metadata::LocalRefName::new(
            "refs/heads/main".try_into()?,
        )?),
        target_commit_id: Some(solo_id),
        push_remote: None,
    };
    let out_of_sync_branch = but_workspace::legacy::push::check_out_of_sync_base_target(
        &repo,
        &branch_project_meta,
        "origin",
    )?;
    assert_eq!(
        out_of_sync_branch,
        Some(but_workspace::legacy::push::OutOfSyncBaseTarget {
            path: None,
            ref_name: "refs/heads/main".to_owned(),
            short_name: "main".to_owned(),
            remote_name: "origin".to_owned(),
            is_tag: false,
            target_sha: solo_id.to_string(),
            remote_sha: Some(base_id.to_string()),
        })
    );

    let result_branch = but_workspace::legacy::push::workspace_push_all(
        &repo,
        &workspace,
        &branch_project_meta,
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
    assert!(
        result_branch
            .branch_to_remote
            .iter()
            .any(|(local, remote_ref, short)| {
                local == "main"
                    && remote_ref.as_bstr() == "refs/remotes/origin/main"
                    && short == "main"
            }),
        "out-of-sync branch base should be pushed with workspace_push_all"
    );

    Ok(())
}

#[test]
fn workspace_push_all_pushes_out_of_sync_submodule_branch_and_tag() -> anyhow::Result<()> {
    let (tmp, repo, meta) = fixture("push")?;
    let workdir = repo.workdir().expect("non-bare repo");

    // Create a bare remote and a local submodule repo inside the fixture
    let sub_remote_dir = tmp.path().join("sub-remote.git");
    let sub_init_remote = std::process::Command::new("git")
        .args(["init", "--bare", sub_remote_dir.to_str().unwrap()])
        .status()?;
    assert!(sub_init_remote.success());

    let sub_workdir = workdir.join("sub");
    let sub_init = std::process::Command::new("git")
        .args(["init", sub_workdir.to_str().unwrap()])
        .status()?;
    assert!(sub_init.success());
    for args in [
        vec!["config", "user.name", "Test"],
        vec!["config", "user.email", "test@example.com"],
        vec!["checkout", "-b", "sub-branch"],
        vec![
            "remote",
            "add",
            "origin",
            sub_remote_dir.to_str().unwrap(),
        ],
    ] {
        let st = std::process::Command::new("git")
            .current_dir(&sub_workdir)
            .args(&args)
            .status()?;
        assert!(st.success());
    }
    std::fs::write(sub_workdir.join("sub.txt"), b"submodule v1\n")?;
    for args in [vec!["add", "sub.txt"], vec!["commit", "-m", "sub v1"]] {
        let st = std::process::Command::new("git")
            .current_dir(&sub_workdir)
            .args(&args)
            .status()?;
        assert!(st.success());
    }
    let sub_repo = gix::open(&sub_workdir)?;
    let sub_commit_oid = sub_repo.head_id()?.detach();

    // Write .gitmodules with branch = sub-branch and record the gitlink in `gitbutler/workspace`'s tree
    std::fs::write(
        workdir.join(".gitmodules"),
        format!(
            "[submodule \"sub\"]\n\tpath = sub\n\turl = {}\n\tbranch = sub-branch\n",
            sub_remote_dir.display()
        ),
    )?;
    for args in [
        vec![
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{sub_commit_oid},sub"),
        ],
        vec!["add", ".gitmodules"],
        vec!["commit", "--amend", "--no-edit"],
    ] {
        let st = std::process::Command::new("git")
            .current_dir(workdir)
            .args(&args)
            .status()?;
        assert!(st.success());
    }

    let repo = gix::open(repo.path())?;
    let p_meta = project_meta(&repo)?;

    let out_of_sync =
        but_workspace::legacy::push::check_out_of_sync_submodules(&repo, &p_meta)?;
    assert_eq!(
        out_of_sync,
        vec![but_workspace::legacy::push::OutOfSyncBaseTarget {
            path: Some("sub".to_owned()),
            ref_name: "refs/heads/sub-branch".to_owned(),
            short_name: "sub-branch".to_owned(),
            remote_name: "origin".to_owned(),
            is_tag: false,
            target_sha: sub_commit_oid.to_string(),
            remote_sha: None,
        }]
    );

    let (info, workspace) = head_info(&repo, &meta)?;
    let mut db = but_db::DbHandle::new_at_path(":memory:")?;
    but_workspace::legacy::push::workspace_push_all(
        &repo,
        &workspace,
        &p_meta,
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

    let sub_remote_repo = gix::open(&sub_remote_dir)?;
    let pushed_sub_oid = sub_remote_repo
        .find_reference("refs/heads/sub-branch")?
        .peel_to_commit()?
        .id;
    assert_eq!(
        pushed_sub_oid, sub_commit_oid,
        "submodule branch should be pushed to its remote by workspace_push_all"
    );

    // Also verify tag push when submodule is detached on a tag and `.gitmodules` has no branch
    std::fs::write(
        workdir.join(".gitmodules"),
        format!(
            "[submodule \"sub\"]\n\tpath = sub\n\turl = {}\n",
            sub_remote_dir.display()
        ),
    )?;
    std::fs::write(sub_workdir.join("sub.txt"), b"submodule v2\n")?;
    for args in [
        vec!["checkout", "--detach"],
        vec!["add", "sub.txt"],
        vec!["commit", "-m", "sub v2"],
        vec!["tag", "v2.0.0"],
    ] {
        let st = std::process::Command::new("git")
            .current_dir(&sub_workdir)
            .args(&args)
            .status()?;
        assert!(st.success());
    }
    let sub_repo = gix::open(&sub_workdir)?;
    let sub_tag_oid = sub_repo.head_id()?.detach();
    for args in [
        vec![
            "update-index",
            "--cacheinfo",
            &format!("160000,{sub_tag_oid},sub"),
        ],
        vec!["add", ".gitmodules"],
        vec!["commit", "--amend", "--no-edit"],
    ] {
        let st = std::process::Command::new("git")
            .current_dir(workdir)
            .args(&args)
            .status()?;
        assert!(st.success());
    }

    let repo = gix::open(repo.path())?;
    let out_of_sync_tag =
        but_workspace::legacy::push::check_out_of_sync_submodules(&repo, &p_meta)?;
    assert_eq!(
        out_of_sync_tag,
        vec![but_workspace::legacy::push::OutOfSyncBaseTarget {
            path: Some("sub".to_owned()),
            ref_name: "refs/tags/v2.0.0".to_owned(),
            short_name: "v2.0.0".to_owned(),
            remote_name: "origin".to_owned(),
            is_tag: true,
            target_sha: sub_tag_oid.to_string(),
            remote_sha: None,
        }]
    );

    let (info, workspace) = head_info(&repo, &meta)?;
    but_workspace::legacy::push::workspace_push_all(
        &repo,
        &workspace,
        &p_meta,
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

    let sub_remote_repo = gix::open(&sub_remote_dir)?;
    let pushed_tag_oid = sub_remote_repo
        .find_reference("refs/tags/v2.0.0")?
        .peel_to_commit()?
        .id;
    assert_eq!(
        pushed_tag_oid, sub_tag_oid,
        "submodule tag should be pushed to its remote by workspace_push_all"
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
