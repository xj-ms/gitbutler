//! This module makes an attempt to build a push implementation that is as
//! non-legacy as possible without doing some larger re-plumbing around the
//! gitbutler-git crate.
//!
//! This module should avoid legacy data structures where possible.

use std::str::FromStr as _;

use anyhow::{Context as _, Result};
use bstr::{BStr, BString, ByteSlice, ByteVec};
use but_core::extract_remote_name_and_short_name;
use but_db::DbHandle;
use gitbutler_git::{PushResult, push_with_askpass};
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

    for (segment, ref_name) in branches_to_push(ref_info, branch).rev() {
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

        let gerrit_push_args = gerrit_push_args(
            gerrit_mode,
            local_sha.id,
            target_branch_name.as_ref().map(|name| name.as_bstr()),
            &push_opts,
        )?;
        let push_output = push_with_askpass(
            repo,
            local_sha.id,
            remote_refname.as_ref(),
            with_force,
            force_push_protection && !skip_force_push_protection,
            gerrit_push_args.refspec,
            // Historically we have tried to pass a stackId to the frontend when
            // doing askpass... but as far as I can tell, it's never used in a
            // meaningful way and doesn't seem to actually be required.
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

// [gix-merge-insert-order: 10]
/// A single applied branch whose local reference will be reset to match the upstream workspace commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushedWorkspaceBranchUpdate {
    /// Short branch name (for example `"fork/push-all"`).
    pub name: String,
    /// Current local `refs/heads/<name>` commit SHA, if the branch exists locally.
    pub old_sha: Option<String>,
    /// Target commit SHA from the upstream workspace commit's parent list.
    pub new_sha: String,
}

/// Target reference and commit SHA changes when resetting to an upstream workspace commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushedWorkspaceTargetUpdate {
    /// Previous `gitbutler.project.targetRef` value, if configured.
    pub old_ref_name: Option<String>,
    /// Resolved target reference name (for example `"refs/tags/0.23.3-fork-base"` or `"refs/heads/main"`).
    pub new_ref_name: String,
    /// Current commit SHA that `new_ref_name` peels to locally, if it exists.
    pub old_ref_sha: Option<String>,
    /// Previous `gitbutler.project.targetCommitId` value, if configured.
    pub old_target_commit_id: Option<String>,
    /// New target commit SHA (octopus merge-base of the workspace parents).
    pub new_target_commit_id: String,
}

/// Summary and plan for resetting the local workspace and applied branches to match the pushed
/// upstream workspace commit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushedWorkspaceUpdatePlan {
    /// Full remote-tracking reference for the workspace (for example `"refs/remotes/origin/fork-master"`).
    pub upstream_ref_name: String,
    /// Remote name (for example `"origin"`).
    pub remote_name: String,
    /// Current local `gitbutler/workspace` (or `HEAD`) commit SHA.
    pub old_workspace_sha: String,
    /// Upstream workspace commit SHA on `upstream_ref_name`.
    pub new_workspace_sha: String,
    /// Target reference and target commit update details.
    pub target: PushedWorkspaceTargetUpdate,
    /// Applied branches (in parent order) to be reset.
    pub branches: Vec<PushedWorkspaceBranchUpdate>,
    /// Submodules modified in the upstream workspace whose local checkout differs from the target gitlink.
    pub submodules: Vec<PushedWorkspaceBranchUpdate>,
    /// `true` if there are uncommitted changes in tracked files in the working tree.
    pub has_uncommitted_changes: bool,
    /// Paths of modified/uncommitted tracked files in the working tree.
    pub uncommitted_files: Vec<String>,
}

/// Resolve `(remote_name, upstream_ref_name, old_workspace_sha, new_workspace_sha)` when
/// `gitbutler/workspace` has a configured remote-tracking branch whose tip differs from the local
/// workspace commit and is a valid managed `GitButler Workspace Commit`.
fn resolve_diverged_pushed_workspace(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
) -> Result<Option<(String, gix::refs::FullName, gix::ObjectId, gix::ObjectId)>> {
    let Some(remote_branch_short) = workspace_remote_branch(repo)? else {
        return Ok(None);
    };
    let local_config = open_repo_local_config_for_reading(repo)?;
    let remote_name = local_config
        .string(WORKSPACE_REMOTE_CONFIG_KEY)
        .map(|s| s.to_str_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| resolve_push_remote(repo, project_meta).ok());
    let Some(remote_name) = remote_name else {
        return Ok(None);
    };

    let upstream_ref_name: gix::refs::FullName =
        match format!("refs/remotes/{remote_name}/{remote_branch_short}").try_into() {
            Ok(name) => name,
            Err(_) => return Ok(None),
        };
    let Some(mut upstream_ref) = repo.try_find_reference(upstream_ref_name.as_ref())? else {
        return Ok(None);
    };
    let Ok(new_workspace_sha) = upstream_ref.peel_to_commit().map(|c| c.id) else {
        return Ok(None);
    };

    let old_workspace_sha = repo
        .try_find_reference(WORKSPACE_REF_NAME)?
        .map(|mut r| r.peel_to_commit().map(|c| c.id))
        .transpose()?
        .or_else(|| repo.head_id().ok().map(|id| id.detach()));
    let Some(old_workspace_sha) = old_workspace_sha else {
        return Ok(None);
    };

    if old_workspace_sha == new_workspace_sha {
        return Ok(None);
    }

    let is_managed = WorkspaceCommit::from_id(new_workspace_sha.attach(repo))
        .is_ok_and(|commit| commit.is_managed());
    if !is_managed {
        return Ok(None);
    }

    Ok(Some((
        remote_name,
        upstream_ref_name,
        old_workspace_sha,
        new_workspace_sha,
    )))
}

/// Fast eligibility check: returns `Some(upstream_seconds - local_seconds)` when
/// `gitbutler/workspace`'s configured remote-tracking branch exists, points to a different commit
/// than local `gitbutler/workspace`, and is a valid managed `GitButler Workspace Commit`.
pub fn pushed_workspace_time_diff_secs(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
) -> Result<Option<i64>> {
    let Some((_, _, old_workspace_sha, new_workspace_sha)) =
        resolve_diverged_pushed_workspace(repo, project_meta)?
    else {
        return Ok(None);
    };
    let old_secs = repo
        .find_commit(old_workspace_sha)?
        .time()
        .map(|t| t.seconds)
        .unwrap_or(0);
    let new_secs = repo
        .find_commit(new_workspace_sha)?
        .time()
        .map(|t| t.seconds)
        .unwrap_or(0);
    Ok(Some(new_secs.saturating_sub(old_secs)))
}

/// Build the [`PushedWorkspaceUpdatePlan`] for a given `new_workspace_sha` commit when it is a
/// valid managed `GitButler Workspace Commit`.
fn build_pushed_workspace_plan_for_commit(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
    remote_name: String,
    upstream_ref_name: String,
    old_workspace_sha: String,
    new_workspace_sha: gix::ObjectId,
    excluded_target_branch_short: Option<&str>,
) -> Result<Option<PushedWorkspaceUpdatePlan>> {
    let Ok(ws_commit) = WorkspaceCommit::from_id(new_workspace_sha.attach(repo)) else {
        return Ok(None);
    };
    if !ws_commit.is_managed() {
        return Ok(None);
    }

    let parents: Vec<gix::ObjectId> = ws_commit.parents.iter().copied().collect();
    if parents.is_empty() {
        anyhow::bail!("Workspace commit {new_workspace_sha} has no parent commits");
    }

    // Build map of commit SHA -> remote branch short names under `refs/remotes/<remote_name>/`
    let remote_prefix = format!("refs/remotes/{remote_name}/");
    let mut remote_branches_by_sha: std::collections::HashMap<gix::ObjectId, Vec<String>> =
        std::collections::HashMap::new();
    if let Ok(platform) = repo.references()
        && let Ok(iter) = platform.prefixed(remote_prefix.as_str())
    {
        for reference in iter.flatten() {
            let full_name = reference.name().as_bstr().to_str_lossy().into_owned();
            if full_name == format!("{remote_prefix}HEAD") || full_name == upstream_ref_name {
                continue;
            }
            let Some(short_name) = full_name.strip_prefix(&remote_prefix) else {
                continue;
            };
            if excluded_target_branch_short.is_some_and(|excl| excl == short_name) {
                continue;
            }
            let mut reference = reference;
            if let Ok(commit) = reference.peel_to_commit() {
                remote_branches_by_sha
                    .entry(commit.id)
                    .or_default()
                    .push(short_name.to_owned());
            }
        }
    }
    for list in remote_branches_by_sha.values_mut() {
        list.sort();
    }

    // Parse workspace commit message for `- <branch>` and `branch head: <sha>` entries
    let message_str = ws_commit.message.to_str_lossy();
    let mut msg_branches_in_order: Vec<String> = Vec::new();
    let mut msg_branch_by_sha: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut last_msg_branch: Option<String> = None;
    for line in message_str.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let branch_name = rest
                .split_once(" (refs/heads/")
                .map(|(b, _)| b.trim())
                .unwrap_or_else(|| rest.trim());
            if !branch_name.is_empty() {
                last_msg_branch = Some(branch_name.to_owned());
                msg_branches_in_order.push(branch_name.to_owned());
            }
        } else if let Some(sha_part) = trimmed.strip_prefix("branch head:")
            && let Some(branch_name) = last_msg_branch.take()
        {
            let sha_str = sha_part.trim();
            if !sha_str.is_empty() {
                msg_branch_by_sha.insert(sha_str.to_owned(), branch_name);
            }
        }
    }

    let mut used_branches: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut branches: Vec<PushedWorkspaceBranchUpdate> = Vec::with_capacity(parents.len());
    for (idx, parent_sha) in parents.iter().copied().enumerate() {
        let parent_hex = parent_sha.to_string();
        let mut resolved: Option<String> = None;

        if let Some(candidates) = remote_branches_by_sha.get(&parent_sha) {
            let available: Vec<&String> = candidates
                .iter()
                .filter(|b| !used_branches.contains(*b))
                .collect();
            if let [single] = available.as_slice() {
                resolved = Some((*single).clone());
            } else if let Some(first) = available.first() {
                if let Some(msg_branch) = msg_branch_by_sha.get(&parent_hex)
                    && available.contains(&msg_branch)
                {
                    resolved = Some(msg_branch.clone());
                } else {
                    resolved = Some((*first).clone());
                }
            }
        }

        if resolved.is_none()
            && let Some(msg_branch) = msg_branch_by_sha.get(&parent_hex)
            && !used_branches.contains(msg_branch)
        {
            resolved = Some(msg_branch.clone());
        }

        if resolved.is_none()
            && msg_branches_in_order.len() == parents.len()
            && let Some(pos_branch) = msg_branches_in_order.get(idx)
            && !used_branches.contains(pos_branch)
        {
            resolved = Some(pos_branch.clone());
        }

        let branch_name = resolved.with_context(|| {
            format!(
                "Could not resolve branch name for parent [{idx}] ({parent_sha}) of workspace commit {new_workspace_sha}"
            )
        })?;
        used_branches.insert(branch_name.clone());

        let local_ref_name = format!("refs/heads/{branch_name}");
        let old_sha = repo
            .try_find_reference(local_ref_name.as_str())?
            .and_then(|mut r| r.peel_to_commit().ok().map(|c| c.id.to_string()));

        branches.push(PushedWorkspaceBranchUpdate {
            name: branch_name,
            old_sha,
            new_sha: parent_hex,
        });
    }

    // Resolve target commit ID via octopus merge-base of parents
    let old_target_ref = project_meta.target_ref.as_ref().map(|t| t.to_string());
    let old_target_commit_id = project_meta.target_commit_id.map(|id| id.to_string());
    let new_target_oid = if parents.len() >= 2 {
        repo.merge_base_octopus(parents.iter().copied())?
            .map(|id| id.detach())
            .or(project_meta.target_commit_id)
    } else {
        project_meta
            .target_commit_id
            .or_else(|| parents.first().copied())
    }
    .context("Could not determine target commit ID for workspace update")?;

    // Resolve target ref name
    let applied_branch_names: std::collections::HashSet<&str> =
        branches.iter().map(|b| b.name.as_str()).collect();
    let new_ref_name = {
        let mut tags_at_target: Vec<String> = Vec::new();
        if let Ok(platform) = repo.references()
            && let Ok(iter) = platform.prefixed("refs/tags/")
        {
            for reference in iter.flatten() {
                let full_name = reference.name().as_bstr().to_str_lossy().into_owned();
                let mut reference = reference;
                if let Ok(commit) = reference.peel_to_commit()
                    && commit.id == new_target_oid
                {
                    tags_at_target.push(full_name);
                }
            }
        }
        tags_at_target.sort();

        if let Some(first_tag) = tags_at_target.first() {
            if let Some(old_ref) = old_target_ref.as_ref()
                && tags_at_target.contains(old_ref)
            {
                old_ref.clone()
            } else {
                first_tag.clone()
            }
        } else if let Some(old_ref) = old_target_ref.clone() {
            old_ref
        } else {
            let mut local_branches_at_target: Vec<String> = Vec::new();
            if let Ok(platform) = repo.references()
                && let Ok(iter) = platform.prefixed("refs/heads/")
            {
                for reference in iter.flatten() {
                    let full_name = reference.name().as_bstr().to_str_lossy().into_owned();
                    let Some(short_name) = full_name.strip_prefix("refs/heads/") else {
                        continue;
                    };
                    if full_name == WORKSPACE_REF_NAME
                        || excluded_target_branch_short.is_some_and(|excl| excl == short_name)
                        || applied_branch_names.contains(short_name)
                    {
                        continue;
                    }
                    let mut reference = reference;
                    if let Ok(commit) = reference.peel_to_commit()
                        && commit.id == new_target_oid
                    {
                        local_branches_at_target.push(full_name);
                    }
                }
            }
            local_branches_at_target.sort();
            if let Some(first_local) = local_branches_at_target.first() {
                first_local.clone()
            } else if let Ok(Some(inferred)) = crate::init::infer_default_target_ref(repo)
                && excluded_target_branch_short
                    .is_none_or(|excl| inferred.as_local_full_name_ref().shorten() != excl)
            {
                inferred.to_string()
            } else {
                "refs/heads/master".to_owned()
            }
        }
    };

    let old_ref_sha = repo
        .try_find_reference(new_ref_name.as_str())?
        .and_then(|mut r| r.peel_to_commit().ok().map(|c| c.id.to_string()));

    let mut submodules = Vec::new();
    if new_target_oid != new_workspace_sha
        && let Ok(changes) =
            but_core::diff::tree_changes(repo, Some(new_target_oid), new_workspace_sha)
    {
        for change in changes {
            let Some(state) = change.status.state() else {
                continue;
            };
            if state.kind != gix::object::tree::EntryKind::Commit {
                continue;
            }
            let sub_path = change.path.to_str_lossy().into_owned();
            let new_sha = state.id.to_string();
            let old_sha = repo
                .workdir()
                .and_then(|wd| gix::open(wd.join(&sub_path)).ok())
                .and_then(|sub_repo| sub_repo.head_id().ok().map(|id| id.detach().to_string()));
            if old_sha.as_deref() != Some(new_sha.as_str()) {
                submodules.push(PushedWorkspaceBranchUpdate {
                    name: sub_path,
                    old_sha,
                    new_sha,
                });
            }
        }
    }

    let uncommitted_files: Vec<String> = but_core::diff::worktree_changes_no_renames(repo)?
        .changes
        .into_iter()
        .filter(|change| {
            !matches!(
                change.status,
                but_core::TreeStatus::Addition {
                    is_untracked: true,
                    ..
                }
            )
        })
        .map(|change| change.path.to_str_lossy().into_owned())
        .collect();
    let has_uncommitted_changes = !uncommitted_files.is_empty();

    Ok(Some(PushedWorkspaceUpdatePlan {
        upstream_ref_name,
        remote_name,
        old_workspace_sha,
        new_workspace_sha: new_workspace_sha.to_string(),
        target: PushedWorkspaceTargetUpdate {
            old_ref_name: old_target_ref,
            new_ref_name,
            old_ref_sha,
            old_target_commit_id,
            new_target_commit_id: new_target_oid.to_string(),
        },
        branches,
        submodules,
        has_uncommitted_changes,
        uncommitted_files,
    }))
}

/// Build the full [`PushedWorkspaceUpdatePlan`] when `gitbutler/workspace`'s remote-tracking
/// branch points to a newer/different managed workspace commit.
pub fn pushed_workspace_update_plan(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
) -> Result<Option<PushedWorkspaceUpdatePlan>> {
    let Some((remote_name, upstream_ref_name, old_workspace_sha, new_workspace_sha)) =
        resolve_diverged_pushed_workspace(repo, project_meta)?
    else {
        return Ok(None);
    };
    let excluded_short = workspace_remote_branch(repo)?;
    build_pushed_workspace_plan_for_commit(
        repo,
        project_meta,
        remote_name,
        upstream_ref_name.to_string(),
        old_workspace_sha.to_string(),
        new_workspace_sha,
        excluded_short.as_deref(),
    )
}

/// Build a [`PushedWorkspaceUpdatePlan`] when setting up a workspace from an arbitrary Git
/// reference (such as a local branch `refs/heads/fork-master` or remote-tracking branch
/// `refs/remotes/origin/fork-master`) if that reference (or its upstream tracking branch) points to
/// a valid managed `GitButler Workspace Commit`.
pub fn pushed_workspace_setup_plan_for_ref(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
    workspace_ref_name: &gix::refs::FullNameRef,
    push_remote: Option<&str>,
) -> Result<Option<PushedWorkspaceUpdatePlan>> {
    if workspace_ref_name.category() == Some(gix::refs::Category::Tag) {
        return Ok(None);
    }
    let Some(mut reference) = repo.try_find_reference(workspace_ref_name)? else {
        return Ok(None);
    };
    let Ok(ref_commit_id) = reference.peel_to_commit().map(|c| c.id) else {
        return Ok(None);
    };

    let mut candidate_commit_id = ref_commit_id;
    let mut resolved_upstream_ref = workspace_ref_name.to_string();
    let mut resolved_remote: Option<String> =
        push_remote.filter(|s| !s.is_empty()).map(ToOwned::to_owned);
    let mut workspace_branch_short: Option<String> = None;

    match workspace_ref_name.category() {
        Some(gix::refs::Category::RemoteBranch) => {
            if let Some((remote, short_name)) = but_core::extract_remote_name_and_short_name(
                workspace_ref_name,
                &repo.remote_names(),
            ) {
                if resolved_remote.is_none() {
                    resolved_remote = Some(remote);
                }
                workspace_branch_short = Some(short_name.to_str_lossy().into_owned());
            }
        }
        Some(gix::refs::Category::LocalBranch) => {
            let short = workspace_ref_name.shorten().to_str_lossy().into_owned();
            workspace_branch_short = Some(short.clone());
            if let Some(Ok(tracking_ref)) = repo
                .branch_remote_tracking_ref_name(workspace_ref_name, gix::remote::Direction::Fetch)
            {
                if let Some((remote, _)) = but_core::extract_remote_name_and_short_name(
                    tracking_ref.as_ref(),
                    &repo.remote_names(),
                ) && resolved_remote.is_none()
                {
                    resolved_remote = Some(remote);
                }
                if !WorkspaceCommit::from_id(candidate_commit_id.attach(repo))
                    .is_ok_and(|c| c.is_managed())
                    && let Ok(Some(mut tr)) = repo.try_find_reference(tracking_ref.as_ref())
                    && let Ok(c) = tr.peel_to_commit()
                    && WorkspaceCommit::from_id(c.id.attach(repo)).is_ok_and(|wc| wc.is_managed())
                {
                    candidate_commit_id = c.id;
                    resolved_upstream_ref = tracking_ref.to_string();
                } else if let Some(ref rem) = resolved_remote {
                    let remote_candidate = format!("refs/remotes/{rem}/{short}");
                    if repo
                        .try_find_reference(remote_candidate.as_str())?
                        .is_some()
                    {
                        resolved_upstream_ref = remote_candidate;
                    }
                }
            }
        }
        _ => {}
    }

    let remote_name = resolved_remote
        .or_else(|| resolve_push_remote(repo, project_meta).ok())
        .unwrap_or_else(|| "origin".to_owned());

    if let Some(ref short) = workspace_branch_short {
        let remote_candidate = format!("refs/remotes/{remote_name}/{short}");
        if resolved_upstream_ref.starts_with("refs/heads/")
            && repo
                .try_find_reference(remote_candidate.as_str())?
                .is_some()
        {
            resolved_upstream_ref = remote_candidate;
        }
    }

    let old_workspace_sha = repo
        .try_find_reference(WORKSPACE_REF_NAME)?
        .and_then(|mut r| r.peel_to_commit().ok().map(|c| c.id.to_string()))
        .or_else(|| repo.head_id().ok().map(|id| id.detach().to_string()))
        .unwrap_or_else(|| candidate_commit_id.to_string());

    build_pushed_workspace_plan_for_commit(
        repo,
        project_meta,
        remote_name,
        resolved_upstream_ref,
        old_workspace_sha,
        candidate_commit_id,
        workspace_branch_short.as_deref(),
    )
}

fn apply_pushed_workspace_plan(
    repo: &gix::Repository,
    plan: &PushedWorkspaceUpdatePlan,
    meta: &mut impl but_core::RefMetadata,
    switch_head_to_workspace: bool,
    workspace_tracking_branch_short: Option<&str>,
) -> Result<ProjectMeta> {
    if plan.has_uncommitted_changes {
        anyhow::bail!(
            "Cannot update workspace because the working tree has uncommitted changes: {}",
            plan.uncommitted_files.join(", ")
        );
    }

    let new_workspace_oid = gix::ObjectId::from_str(&plan.new_workspace_sha)?;
    let new_target_oid = gix::ObjectId::from_str(&plan.target.new_target_commit_id)?;

    // 1. Update local branch refs and tracking config
    for branch in &plan.branches {
        let parent_oid = gix::ObjectId::from_str(&branch.new_sha)?;
        let local_ref: gix::refs::FullName = format!("refs/heads/{}", branch.name).try_into()?;
        repo.reference(
            local_ref.as_ref(),
            parent_oid,
            gix::refs::transaction::PreviousValue::Any,
            "reset applied branch to pushed workspace parent",
        )?;

        let remote_ref_str = format!("refs/remotes/{}/{}", plan.remote_name, branch.name);
        if let Ok(remote_ref) = gix::refs::FullName::try_from(remote_ref_str)
            && repo.try_find_reference(remote_ref.as_ref())?.is_some()
        {
            track_pushed_branch(repo, local_ref.as_ref(), remote_ref.as_ref());
        }
    }

    // 2. Update the target Git ref if needed
    if plan.target.old_ref_sha.as_deref() != Some(plan.target.new_target_commit_id.as_str()) {
        let target_full_name: gix::refs::FullName = plan.target.new_ref_name.clone().try_into()?;
        repo.reference(
            target_full_name.as_ref(),
            new_target_oid,
            gix::refs::transaction::PreviousValue::Any,
            "sync workspace base target ref",
        )?;
    }

    let local_target_ref =
        but_core::ref_metadata::LocalRefName::new(plan.target.new_ref_name.clone().try_into()?)?;
    let is_tag = local_target_ref.is_tag();
    let updated_project_meta = ProjectMeta {
        target_ref: Some(local_target_ref),
        target_commit_id: Some(new_target_oid),
        push_remote: if is_tag && plan.remote_name.is_empty() {
            None
        } else {
            Some(plan.remote_name.clone())
        },
    };

    // 3. Checkout the new workspace commit and update `refs/heads/gitbutler/workspace`
    but_core::worktree::safe_checkout_from_head(
        new_workspace_oid,
        repo,
        but_core::worktree::checkout::Options {
            skip_head_update: true,
            ..Default::default()
        },
    )?;
    repo.reference(
        WORKSPACE_REF_NAME,
        new_workspace_oid,
        gix::refs::transaction::PreviousValue::Any,
        "reset workspace head to pushed upstream workspace commit",
    )?;
    if switch_head_to_workspace {
        let ws_full_name: gix::refs::FullName = WORKSPACE_REF_NAME.try_into()?;
        but_core::update_head_reference(
            repo,
            gix::refs::Target::Symbolic(ws_full_name),
            false,
            "setup pushed workspace",
            WORKSPACE_REF_NAME.into(),
            plan.branches.len(),
        )?;
    }
    if let Some(short_branch) = workspace_tracking_branch_short
        && !plan.remote_name.is_empty()
    {
        let _ = set_workspace_remote_tracking_branch(repo, &plan.remote_name, short_branch);
    }

    // 3b. Update modified submodules whose gitlinks changed in the workspace
    if !plan.submodules.is_empty()
        && let Some(workdir) = repo.workdir()
    {
        for sub in &plan.submodules {
            let sub_workdir = workdir.join(&sub.name);
            if let Ok(sub_repo) = gix::open(&sub_workdir)
                && let Ok(remote_name) = resolve_submodule_push_remote(&sub_repo)
            {
                let _ = gitbutler_git::fetch_with_askpass(
                    &sub_workdir,
                    &remote_name,
                    Some("submodule".to_owned()),
                );
            }
        }
        let git_exe = gix::path::env::exe_invocation();
        let mut cmd = std::process::Command::new(git_exe);
        cmd.current_dir(workdir)
            .args(["submodule", "update", "--init", "--filter=blob:none", "--"]);
        for sub in &plan.submodules {
            cmd.arg(&sub.name);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let output = cmd
            .output()
            .context("failed to run `git submodule update`")?;
        if !output.status.success() {
            anyhow::bail!(
                "`git submodule update` failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }

    edit_repo_config(repo, gix::config::Source::Local, |config| {
        but_core::git_config::ensure_config_value(
            config,
            "log.excludeDecoration",
            "refs/gitbutler",
        )
        .context("failed to set log.excludeDecoration")?;
        Ok(())
    })?;

    // 4. Update workspace metadata (`virtual_branches.toml` + `BranchOrderMetadata`)
    let mut ws_meta = meta.workspace(WORKSPACE_REF_NAME.try_into()?)?;
    let mut existing_ids_by_ref: std::collections::HashMap<
        gix::refs::FullName,
        but_core::ref_metadata::StackId,
    > = std::collections::HashMap::new();
    for stack in &ws_meta.stacks {
        for segment in &stack.branches {
            existing_ids_by_ref
                .entry(segment.ref_name.clone())
                .or_insert(stack.id);
        }
    }
    let mut used_stack_ids: std::collections::HashSet<but_core::ref_metadata::StackId> =
        std::collections::HashSet::new();
    let mut new_stacks = Vec::with_capacity(plan.branches.len());
    for branch in &plan.branches {
        let ref_name: gix::refs::FullName = format!("refs/heads/{}", branch.name).try_into()?;
        let stack_id = existing_ids_by_ref
            .get(&ref_name)
            .copied()
            .filter(|id| !used_stack_ids.contains(id))
            .unwrap_or_else(but_core::ref_metadata::StackId::generate);
        used_stack_ids.insert(stack_id);
        new_stacks.push(but_core::ref_metadata::WorkspaceStack {
            id: stack_id,
            branches: vec![but_core::ref_metadata::WorkspaceStackBranch {
                ref_name,
                archived: false,
            }],
            workspacecommit_relation: but_core::ref_metadata::WorkspaceCommitRelation::Merged,
        });
    }
    ws_meta.stacks = new_stacks;
    meta.set_workspace(&ws_meta)?;

    Ok(updated_project_meta)
}

/// Execute the pushed-workspace reset:
/// 1. Rejects if there are uncommitted worktree changes.
/// 2. Resets local branch refs (`refs/heads/<branch>`) and upstream tracking config.
/// 3. Updates the target ref (`new_ref_name`) and returns the updated [`ProjectMeta`] and
///    [`PushedWorkspaceUpdatePlan`].
/// 4. Checks out the upstream workspace commit tree and updates `refs/heads/gitbutler/workspace`.
pub fn execute_pushed_workspace_update(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
    meta: &mut impl but_core::RefMetadata,
) -> Result<(PushedWorkspaceUpdatePlan, ProjectMeta)> {
    let plan = pushed_workspace_update_plan(repo, project_meta)?
        .context("Workspace is already up to date with its remote tracking branch")?;
    let updated_project_meta = apply_pushed_workspace_plan(repo, &plan, meta, false, None)?;
    Ok((plan, updated_project_meta))
}

/// Set up a GitButler workspace from a reference pointing to a pushed `GitButler Workspace Commit`:
/// 1. Resolves the plan via [`pushed_workspace_setup_plan_for_ref`].
/// 2. Rejects if there are uncommitted tracked changes in the working tree.
/// 3. Creates/resets all applied branch refs, updates the resolved base target ref & [`ProjectMeta`],
///    checks out `refs/heads/gitbutler/workspace` (`HEAD`), configures `branch.gitbutler/workspace`
///    remote tracking, and writes workspace stack metadata.
pub fn execute_pushed_workspace_setup_from_ref(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
    workspace_ref_name: &gix::refs::FullNameRef,
    push_remote: Option<&str>,
    meta: &mut impl but_core::RefMetadata,
) -> Result<(PushedWorkspaceUpdatePlan, ProjectMeta)> {
    let plan =
        pushed_workspace_setup_plan_for_ref(repo, project_meta, workspace_ref_name, push_remote)?
            .with_context(|| {
            format!(
                "Reference '{}' does not point to a valid GitButler workspace commit",
                workspace_ref_name.as_bstr()
            )
        })?;
    let short_branch = match workspace_ref_name.category() {
        Some(gix::refs::Category::RemoteBranch) => {
            but_core::extract_remote_name_and_short_name(workspace_ref_name, &repo.remote_names())
                .map(|(_, short)| short.to_str_lossy().into_owned())
        }
        Some(gix::refs::Category::LocalBranch) => {
            Some(workspace_ref_name.shorten().to_str_lossy().into_owned())
        }
        _ => None,
    };
    let updated_project_meta =
        apply_pushed_workspace_plan(repo, &plan, meta, true, short_branch.as_deref())?;
    Ok((plan, updated_project_meta))
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
