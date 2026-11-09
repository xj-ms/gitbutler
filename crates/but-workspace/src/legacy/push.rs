//! This module makes an attempt to build a push implementation that is as
//! non-legacy as possible without doing some larger re-plumbing around the
//! gitbutler-git crate.
//!
//! This module should avoid legacy data structures where possible.

use std::str::FromStr as _;

use anyhow::{Context as _, Result};
use bstr::{BStr, BString, ByteSlice, ByteVec};
use but_core::{
    WORKSPACE_REF_NAME, extract_remote_name_and_short_name,
    git_config::{edit_repo_config, open_repo_local_config_for_reading, set_config_value},
    ref_metadata::ProjectMeta,
};
use but_db::DbHandle;
use gitbutler_git::{PushResult, push_refspecs_with_askpass, push_with_askpass};
use gitbutler_reference::RemoteRefname;
use gitbutler_repo::hooks;
use gix::{prelude::ObjectIdExt as _, refs::Category};
use indexmap::IndexMap;

use crate::{
    RefInfo, WorkspaceCommit,
    ref_info::{LocalCommitRelation, Segment},
    ui::PushStatus,
};

/// Push a given branch and its ancestors.
///
/// Refuses before pushing anything when that would publish GitButler's workspace commit.
#[allow(clippy::too_many_arguments)]
pub fn workspace_branch_and_ancestors_push(
    repo: &gix::Repository,
    ws: &but_graph::Workspace,
    ref_info: &RefInfo,
    db: &mut DbHandle,
    gerrit_mode: bool,
    with_force: bool,
    skip_force_push_protection: bool,
    force_push_protection: bool,
    branch: &gix::refs::FullNameRef,
    run_hooks: bool,
    run_husky_hooks: bool,
    push_opts: Vec<but_gerrit::PushFlag>,
) -> Result<PushResult> {
    let graph = &ws.graph;
    if pushes_workspace_commit(repo, ref_info, branch) {
        anyhow::bail!(
            "{} is built on GitButler's workspace commit, which merges every applied branch, so \
             pushing it would publish all of them. Rebase its own commits onto the target branch \
             first.",
            branch.shorten()
        );
    }

    let remote_names = repo.remote_names();
    let target_branch_name = if gerrit_mode {
        let upstream = ws
            .graph
            .project_meta
            .remote_target_ref(repo)?
            .context("Gerrit push requires a target upstream")?;
        Some(target_branch_name_from_ref_name(
            upstream.as_ref(),
            &remote_names,
        )?)
    } else {
        None
    };
    let push_remote = ws
        .push_remote_name(repo)?
        .context("failed to get target push remote name")?;
    let mut result = PushResult {
        remote: push_remote.clone(),
        branch_to_remote: vec![],
        branch_sha_updates: vec![],
    };

    for (segment, _ref_name) in branches_to_push(ref_info, branch).rev() {
        push_segment(
            repo,
            graph,
            db,
            &remote_names,
            &push_remote,
            target_branch_name.as_ref().map(|name| name.as_bstr()),
            gerrit_mode,
            with_force,
            skip_force_push_protection,
            force_push_protection,
            run_hooks,
            run_husky_hooks,
            &push_opts,
            segment.id,
            segment,
            &mut result,
        )?;
    }

    Ok(result)
}

const WORKSPACE_REMOTE_CONFIG_KEY: &str = "branch.gitbutler/workspace.remote";
const WORKSPACE_MERGE_CONFIG_KEY: &str = "branch.gitbutler/workspace.merge";

/// Return the remote name to push workspace and stack branches to.
///
/// Prefers `project_meta.push_remote_name(repo)`, falling back to any remote configured for
/// `gitbutler/workspace`, then the repository's default push/fetch remote, and finally `origin`
/// or the sole configured remote.
pub fn resolve_push_remote(repo: &gix::Repository, project_meta: &ProjectMeta) -> Result<String> {
    if let Ok(push_remote) = project_meta.push_remote_name(repo) {
        return Ok(push_remote);
    }

    let local_config = open_repo_local_config_for_reading(repo)?;
    if let Some(remote) = local_config.string(WORKSPACE_REMOTE_CONFIG_KEY) {
        let remote = remote.to_str_lossy().into_owned();
        if !remote.is_empty() && repo.find_remote(remote.as_str()).is_ok() {
            return Ok(remote);
        }
    }

    for direction in [gix::remote::Direction::Push, gix::remote::Direction::Fetch] {
        if let Some(remote) = repo.remote_default_name(direction) {
            return Ok(remote.to_str_lossy().into_owned());
        }
    }

    let remote_names = repo.remote_names();
    if remote_names.iter().any(|name| name.as_bstr() == b"origin") {
        return Ok("origin".to_owned());
    }

    if remote_names.len() == 1
        && let Some(only) = remote_names.into_iter().next()
    {
        return Ok(only.to_str_lossy().into_owned());
    }

    anyhow::bail!("failed to determine push remote name")
}

/// Return the configured remote-tracking branch short name for `gitbutler/workspace`, if any.
pub fn workspace_remote_branch(repo: &gix::Repository) -> Result<Option<String>> {
    let config = open_repo_local_config_for_reading(repo)?;
    let Some(merge_ref) = config.string(WORKSPACE_MERGE_CONFIG_KEY) else {
        return Ok(None);
    };
    let merge_ref = merge_ref.to_str_lossy();
    let short_name = merge_ref.strip_prefix("refs/heads/").unwrap_or(&merge_ref);
    if short_name.is_empty() {
        return Ok(None);
    }
    Ok(Some(short_name.to_owned()))
}

/// Persist the remote tracking configuration (`remote` and `merge`) for `gitbutler/workspace`
/// in the repository-local Git config.
fn set_workspace_remote_tracking_branch(
    repo: &gix::Repository,
    remote_name: &str,
    remote_branch_short_name: &str,
) -> Result<()> {
    let normalized = but_core::branch::normalize_short_name(remote_branch_short_name)?;
    let merge_ref = format!("refs/heads/{normalized}");
    edit_repo_config(repo, gix::config::Source::Local, |config| {
        set_config_value(config, WORKSPACE_REMOTE_CONFIG_KEY, remote_name)?;
        set_config_value(config, WORKSPACE_MERGE_CONFIG_KEY, &merge_ref)?;
        Ok(())
    })?;
    Ok(())
}

/// Push `gitbutler/workspace` to `workspace_remote_branch` on the default remote (persisting its
/// tracking configuration in local Git config), along with all applied branches that are out of sync.
#[allow(clippy::too_many_arguments)]
pub fn workspace_push_all(
    repo: &gix::Repository,
    ws: &but_graph::Workspace,
    project_meta: &ProjectMeta,
    ref_info: &RefInfo,
    db: &mut DbHandle,
    gerrit_mode: bool,
    workspace_remote_branch: &str,
    skip_force_push_protection: bool,
    force_push_protection: bool,
    run_hooks: bool,
    run_husky_hooks: bool,
    push_opts: Vec<but_gerrit::PushFlag>,
) -> Result<PushResult> {
    let graph = &ws.graph;
    let remote_names = repo.remote_names();
    let target_ref_name = ws.target_ref_name().map(ToOwned::to_owned);
    let push_remote = resolve_push_remote(repo, project_meta)?;
    let normalized_workspace_branch =
        but_core::branch::normalize_short_name(workspace_remote_branch)?.to_string();
    set_workspace_remote_tracking_branch(repo, &push_remote, &normalized_workspace_branch)?;

    let target_branch_name = match target_ref_name.as_ref() {
        Some(target_ref_name) => {
            target_branch_name_from_ref_name(target_ref_name.as_ref(), &remote_names)?
        }
        None => BString::from(normalized_workspace_branch.as_str()),
    };

    let mut result = PushResult {
        remote: push_remote.clone(),
        branch_to_remote: vec![],
        branch_sha_updates: vec![],
    };

    // 1. Prepare the workspace head (`gitbutler/workspace`) push
    let workspace_head_sha = repo
        .try_find_reference(WORKSPACE_REF_NAME)?
        .map(|mut r| r.peel_to_commit().map(|c| c.id))
        .transpose()?
        .or_else(|| graph.tip_skip_empty(ws.id).map(|c| c.id))
        .context("failed to resolve workspace head commit for `gitbutler/workspace`")?;
    let workspace_remote_refname: gix::refs::FullName =
        format!("refs/remotes/{push_remote}/{normalized_workspace_branch}").try_into()?;
    let workspace_before_sha = remote_before_sha(repo, workspace_remote_refname.as_ref())?;
    let remote = repo.find_remote(push_remote.as_str())?;
    let remote_url = remote
        .url(gix::remote::Direction::Push)
        .or_else(|| remote.url(gix::remote::Direction::Fetch))
        .with_context(|| format!("Remote named {push_remote} didn't have a URL"))?;

    if run_hooks {
        match hooks::pre_push(
            repo,
            &push_remote,
            &remote_url.to_bstring().to_str_lossy(),
            workspace_head_sha,
            &RemoteRefname::from_str(&workspace_remote_refname.as_bstr().to_str_lossy())?,
            run_husky_hooks,
        )? {
            hooks::HookResult::Success | hooks::HookResult::NotConfigured => Ok(()),
            hooks::HookResult::Failure(error_data) => Err(anyhow::anyhow!(
                "pre-push hook failed: {}",
                error_data.error
            )),
        }?;
    }

    let workspace_short_name = WORKSPACE_REF_NAME
        .strip_prefix("refs/heads/")
        .unwrap_or(WORKSPACE_REF_NAME)
        .to_string();

    if gerrit_mode {
        push_with_askpass(
            repo,
            workspace_head_sha,
            workspace_remote_refname.as_ref(),
            true,
            false,
            None,
            Some(None),
            vec![],
        )?;
        result.branch_to_remote.push((
            workspace_short_name.clone(),
            workspace_remote_refname,
            normalized_workspace_branch,
        ));
        result.branch_sha_updates.push((
            workspace_short_name,
            workspace_before_sha.to_string(),
            workspace_head_sha.to_string(),
        ));

        let mut pushed_segments = std::collections::HashSet::new();
        for stack in &ref_info.stacks {
            for segment in stack.segments.iter().rev() {
                let Some(ref_name) = segment.ref_info.as_ref().map(|r| r.ref_name.as_ref()) else {
                    continue;
                };
                if ref_name.category() != Some(Category::LocalBranch) {
                    continue;
                }
                if !pushed_segments.insert(segment.id) {
                    continue;
                }
                if !matches!(
                    segment.push_status,
                    PushStatus::UnpushedCommits
                        | PushStatus::UnpushedCommitsRequiringForce
                        | PushStatus::CompletelyUnpushed
                ) {
                    continue;
                }
                let with_force = matches!(
                    segment.push_status,
                    PushStatus::UnpushedCommitsRequiringForce
                );
                push_segment(
                    repo,
                    graph,
                    db,
                    &remote_names,
                    &push_remote,
                    Some(target_branch_name.as_bstr()),
                    gerrit_mode,
                    with_force,
                    skip_force_push_protection,
                    force_push_protection,
                    run_hooks,
                    run_husky_hooks,
                    &push_opts,
                    segment.id,
                    segment,
                    &mut result,
                )?;
            }
        }

        return Ok(result);
    }

    // 2. Non-Gerrit mode: batch refspecs per remote so a single `git push` handles all branches
    let mut refspecs_by_remote: Vec<(String, Vec<String>)> = vec![(
        push_remote.clone(),
        vec![format!(
            "+{workspace_head_sha}:refs/heads/{normalized_workspace_branch}"
        )],
    )];
    let mut pending_updates = vec![(
        (
            workspace_short_name.clone(),
            workspace_remote_refname,
            normalized_workspace_branch,
        ),
        (
            workspace_short_name,
            workspace_before_sha.to_string(),
            workspace_head_sha.to_string(),
        ),
    )];

    let mut pushed_segments = std::collections::HashSet::new();
    for stack in &ref_info.stacks {
        for segment in stack.segments.iter().rev() {
            let Some(ref_name) = segment.ref_info.as_ref().map(|r| r.ref_name.as_ref()) else {
                continue;
            };
            if ref_name.category() != Some(Category::LocalBranch) {
                continue;
            }
            if !pushed_segments.insert(segment.id) {
                continue;
            }
            if !matches!(
                segment.push_status,
                PushStatus::UnpushedCommits
                    | PushStatus::UnpushedCommitsRequiringForce
                    | PushStatus::CompletelyUnpushed
            ) {
                continue;
            }
            let Some(local_sha) = graph.tip_skip_empty(segment.id) else {
                continue;
            };
            let remote_refname = match &segment.remote_tracking_ref_name {
                Some(r) => r.clone(),
                None => format_remote_refname(ref_name, &push_remote)?,
            };
            let (remote_name, remote_branch_name) =
                extract_remote_name_and_short_name(remote_refname.as_ref(), &remote_names)
                    .with_context(|| {
                        format!("failed to determine remote name for `{remote_refname}`")
                    })?;
            let remote_branch_str = remote_branch_name.to_str_lossy().to_string();
            let before_sha = remote_before_sha(repo, remote_refname.as_ref())?;
            let seg_remote = repo.find_remote(remote_name.as_str())?;
            let seg_remote_url = seg_remote
                .url(gix::remote::Direction::Push)
                .or_else(|| seg_remote.url(gix::remote::Direction::Fetch))
                .with_context(|| format!("Remote named {remote_name} didn't have a URL"))?;

            if run_hooks {
                match hooks::pre_push(
                    repo,
                    &remote_name,
                    &seg_remote_url.to_bstring().to_str_lossy(),
                    local_sha.id,
                    &RemoteRefname::from_str(&remote_refname.as_bstr().to_str_lossy())?,
                    run_husky_hooks,
                )? {
                    hooks::HookResult::Success | hooks::HookResult::NotConfigured => Ok(()),
                    hooks::HookResult::Failure(error_data) => Err(anyhow::anyhow!(
                        "pre-push hook failed: {}",
                        error_data.error
                    )),
                }?;
            }

            let with_force = matches!(
                segment.push_status,
                PushStatus::UnpushedCommitsRequiringForce
            );
            let prefix = if with_force { "+" } else { "" };
            let refspec = format!("{prefix}{}:refs/heads/{remote_branch_str}", local_sha.id);
            if let Some((_, list)) = refspecs_by_remote
                .iter_mut()
                .find(|(r, _)| r == &remote_name)
            {
                list.push(refspec);
            } else {
                refspecs_by_remote.push((remote_name, vec![refspec]));
            }

            let branch_name = ref_name.shorten().to_str_lossy().to_string();
            pending_updates.push((
                (branch_name.clone(), remote_refname, remote_branch_str),
                (
                    branch_name,
                    before_sha.to_string(),
                    local_sha.id.to_string(),
                ),
            ));
        }
    }

    for (remote_name, refspecs) in refspecs_by_remote {
        push_refspecs_with_askpass(repo, &remote_name, refspecs, false, false, Some(None), vec![])?;
    }

    for (branch_to_remote, sha_update) in pending_updates {
        result.branch_to_remote.push(branch_to_remote);
        result.branch_sha_updates.push(sha_update);
    }

    Ok(result)
}

/// Make `remote_refname`, just pushed, the upstream of `ref_name` where its config names none,
/// as `git push -u` does, so Git outside GitButler knows where the branch goes. The push
/// succeeded, so failing to record that only logs.
fn track_pushed_branch(
    repo: &gix::Repository,
    ref_name: &gix::refs::FullNameRef,
    remote_refname: &gix::refs::FullNameRef,
) {
    if repo
        .branch_remote_tracking_ref_name(ref_name, gix::remote::Direction::Fetch)
        .is_some()
    {
        return;
    }
    let tracked = crate::branch::setup_local_tracking_configuration(repo, ref_name, remote_refname)
        .and_then(|(config, _)| Ok(config.commit()?));
    if let Err(err) = tracked {
        tracing::warn!(
            ?err,
            "couldn't make {remote_refname} the upstream of {ref_name}"
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn push_segment(
    repo: &gix::Repository,
    graph: &but_graph::Graph,
    db: &mut DbHandle,
    remote_names: &gix::remote::Names,
    push_remote: &str,
    target_branch_name: Option<&BStr>,
    gerrit_mode: bool,
    with_force: bool,
    skip_force_push_protection: bool,
    force_push_protection: bool,
    run_hooks: bool,
    run_husky_hooks: bool,
    push_opts: &[but_gerrit::PushFlag],
    sidx: but_graph::SegmentIndex,
    segment: &Segment,
    result: &mut PushResult,
) -> Result<()> {
    // this will always be set
    let Some(ref_name) = segment.ref_info.as_ref().map(|r| r.ref_name.as_ref()) else {
        return Ok(());
    };

    if matches!(
        segment.push_status,
        PushStatus::Integrated | PushStatus::NothingToPush
    ) {
        return Ok(());
    }

    let Some(local_sha) = graph.tip_skip_empty(sidx) else {
        return Ok(());
    };

    let remote_refname = match &segment.remote_tracking_ref_name {
        Some(r) => r.clone(),
        None => format_remote_refname(ref_name, push_remote)?,
    };
    let (remote_name, remote_branch_name) =
        extract_remote_name_and_short_name(remote_refname.as_ref(), remote_names)
            .with_context(|| format!("failed to determine remote name for `{remote_refname}`"))?;
    let before_sha = remote_before_sha(repo, remote_refname.as_ref())?;
    let remote = repo.find_remote(remote_name.as_str())?;
    let remote_url = remote
        .url(gix::remote::Direction::Push)
        .or_else(|| remote.url(gix::remote::Direction::Fetch))
        .with_context(|| format!("Remote named {remote_name} didn't have a URL"))?;

    if run_hooks {
        match hooks::pre_push(
            repo,
            &remote_name,
            &remote_url.to_bstring().to_str_lossy(),
            local_sha.id,
            &RemoteRefname::from_str(&remote_refname.as_bstr().to_str_lossy())?,
            run_husky_hooks,
        )? {
            hooks::HookResult::Success | hooks::HookResult::NotConfigured => Ok(()),
            hooks::HookResult::Failure(error_data) => Err(anyhow::anyhow!(
                "pre-push hook failed: {}",
                error_data.error
            )),
        }?;
    }

    let gerrit_push_args =
        gerrit_push_args(gerrit_mode, local_sha.id, target_branch_name, push_opts)?;
    let push_output = push_with_askpass(
        repo,
        local_sha.id,
        remote_refname.as_ref(),
        with_force,
        force_push_protection && !skip_force_push_protection,
        gerrit_push_args.refspec,
        Some(None),
        gerrit_push_args.push_opts,
    )?;

    maybe_record_gerrit_push_metadata(repo, db, gerrit_mode, segment, &push_output)?;
    if !gerrit_mode {
        track_pushed_branch(repo, ref_name, remote_refname.as_ref());
    }
    let branch_name = ref_name.shorten().to_str_lossy().to_string();
    result.branch_to_remote.push((
        branch_name.clone(),
        remote_refname,
        remote_branch_name.to_str_lossy().to_string(),
    ));
    result.branch_sha_updates.push((
        branch_name,
        before_sha.to_string(),
        local_sha.id.to_string(),
    ));

    Ok(())
}

/// Return the segment of the selected local branch and every segment beneath it in top-to-base
/// order, crossing into the lane a worktree rests on.
///
/// This is the logical scope of a push operation: every commit the push transfers sits in one of
/// these segments, whether or not a branch names it. [`branches_to_push()`] picks the ones pushed
/// as refs.
pub fn branch_and_ancestor_segments<'a>(
    ref_info: &'a RefInfo,
    branch: &gix::refs::FullNameRef,
) -> IndexMap<but_graph::SegmentIndex, &'a Segment> {
    ref_info
        .lane_chain(branch)
        .into_iter()
        .flat_map(|(lane, index)| lane.segments_from(index))
        .map(|segment| (segment.id, segment))
        .collect()
}

/// The segments of [`branch_and_ancestor_segments()`] that pushing `branch` pushes, in top-to-base
/// order, each with the local branch naming it: those not integrated and not already current on
/// the remote.
///
/// [`workspace_branch_and_ancestors_push()`] pushes exactly these, so a preview of the push must
/// list them too.
pub fn branches_to_push<'a>(
    ref_info: &'a RefInfo,
    branch: &gix::refs::FullNameRef,
) -> impl DoubleEndedIterator<Item = (&'a Segment, &'a gix::refs::FullNameRef)> + use<'a> {
    branch_and_ancestor_segments(ref_info, branch)
        .into_values()
        .filter(|segment| {
            !matches!(
                segment.push_status,
                PushStatus::Integrated | PushStatus::NothingToPush
            )
        })
        .filter_map(|segment| {
            let ref_name = segment
                .ref_name()
                .filter(|name| name.category() == Some(Category::LocalBranch))?;
            Some((segment, ref_name))
        })
}

/// Whether pushing `branch` sends GitButler's workspace commit, which merges every applied
/// branch. A branch made with `git checkout -b` on the workspace holds it, and a worktree made
/// from the main checkout's `HEAD`, or one stacked on such a worktree, rests on it. A workspace
/// commit the remote already has publishes nothing new.
fn pushes_workspace_commit(
    repo: &gix::Repository,
    ref_info: &RefInfo,
    branch: &gix::refs::FullNameRef,
) -> bool {
    let managed = |id: gix::ObjectId| {
        WorkspaceCommit::from_id(id.attach(repo)).is_ok_and(|commit| commit.is_managed())
    };
    let chain = ref_info.lane_chain(branch);
    let mut unpushed = chain
        .iter()
        .flat_map(|(lane, index)| lane.segments_from(*index))
        .flat_map(|segment| &segment.commits)
        .filter(|c| !matches!(c.relation, LocalCommitRelation::LocalAndRemote(remote) if remote == c.id));
    unpushed.any(|c| managed(c.id))
        || chain
            .iter()
            .any(|(lane, _)| lane.rests_on.is_some_and(managed))
}

struct GerritPushArgs {
    refspec: Option<String>,
    push_opts: Vec<String>,
}

fn gerrit_push_args(
    gerrit_mode: bool,
    head: gix::ObjectId,
    target_branch_name: Option<&BStr>,
    push_flags: &[but_gerrit::PushFlag],
) -> Result<GerritPushArgs> {
    Ok(if gerrit_mode {
        let target_branch_name =
            target_branch_name.context("Gerrit push requires a target upstream")?;
        GerritPushArgs {
            refspec: Some(format!("{head}:refs/for/{target_branch_name}")),
            push_opts: push_flags.iter().map(|flag| flag.to_string()).collect(),
        }
    } else {
        GerritPushArgs {
            refspec: None,
            push_opts: vec![],
        }
    })
}

/// Derive the target branch name while tolerating stale target remotes.
fn target_branch_name_from_ref_name(
    target_ref_name: &gix::refs::FullNameRef,
    remote_names: &gix::remote::Names,
) -> Result<BString> {
    let (category, shorthand_name) = target_ref_name
        .category_and_short_name()
        .context("Target branch could not be categorized")?;
    if matches!(category, Category::RemoteBranch) {
        if let Some((_remote, short_name)) =
            extract_remote_name_and_short_name(target_ref_name, remote_names)
        {
            return Ok(short_name);
        }
        let remote_ref: RemoteRefname = target_ref_name.to_string().parse()?;
        return Ok(remote_ref.branch().into());
    }
    Ok(shorthand_name.to_owned())
}

fn maybe_record_gerrit_push_metadata(
    repo: &gix::Repository,
    db: &mut DbHandle,
    gerrit_mode: bool,
    segment: &Segment,
    push_output: &str,
) -> Result<()> {
    if !gerrit_mode {
        return Ok(());
    }

    let push_output = but_gerrit::parse::push_output(push_output)?;
    but_gerrit::record_push_metadata(
        repo,
        db,
        segment.commits.iter().map(|c| c.id).collect::<Vec<_>>(),
        push_output,
    )
}

fn format_remote_refname(
    reference: &gix::refs::FullNameRef,
    remote_name: &str,
) -> Result<gix::refs::FullName> {
    let mut out: BString = b"refs/remotes/".into();
    out.push_str(format!("{remote_name}/"));
    out.push_str(reference.shorten());

    Ok(out.try_into()?)
}

fn remote_before_sha(
    repo: &gix::Repository,
    remote_refname: &gix::refs::FullNameRef,
) -> Result<gix::ObjectId> {
    Ok(repo
        .try_find_reference(remote_refname)?
        .map(|mut reference| reference.peel_to_commit())
        .transpose()?
        .map(|commit| commit.id)
        .unwrap_or(repo.object_hash().null()))
}
