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
use gitbutler_git::{
    PushResult, ls_remote_commit_with_askpass, push_refspecs_with_askpass, push_with_askpass,
};
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

/// Information about a workspace base target or a modified submodule whose target ref is not
/// synced with its remote.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutOfSyncBaseTarget {
    /// Submodule path relative to the superproject root (for example `"vendor/gitoxide"`), or
    /// `None` when this entry represents the superproject's workspace base target.
    pub path: Option<String>,
    /// Fully-qualified target reference (for example `"refs/tags/0.23.3-fork-base"` or `"refs/heads/main"`).
    pub ref_name: String,
    /// Display name of the target (for example `"0.23.3-fork-base"` or `"main"`).
    pub short_name: String,
    /// Remote name the target is synced against (for example `"origin"`).
    pub remote_name: String,
    /// `true` if the target is a Git tag (`refs/tags/*`), `false` if it is a branch (`refs/heads/*`).
    pub is_tag: bool,
    /// The local commit SHA the target is pinned to.
    pub target_sha: String,
    /// The commit SHA currently on the remote, if the ref exists there.
    pub remote_sha: Option<String>,
}

fn read_gitmodules_branch(workdir: &std::path::Path, submodule_path: &str) -> Option<String> {
    let gitmodules_path = workdir.join(".gitmodules");
    let config = gix::config::File::from_path_no_includes(
        gitmodules_path,
        gix::config::Source::Local,
    )
    .ok()?;
    if let Some(sections) = config.sections_by_name("submodule") {
        for section in sections {
            let path_matches = section
                .value("path")
                .is_some_and(|p| p.to_str_lossy() == submodule_path)
                || section
                    .header()
                    .subsection_name()
                    .is_some_and(|name| name.to_str_lossy() == submodule_path);
            if path_matches
                && let Some(branch) = section.value("branch")
            {
                let branch = branch.to_str_lossy().trim().to_owned();
                if !branch.is_empty() && branch != "." {
                    return Some(branch);
                }
            }
        }
    }
    None
}

fn resolve_submodule_push_remote(sub_repo: &gix::Repository) -> Result<String> {
    for direction in [gix::remote::Direction::Push, gix::remote::Direction::Fetch] {
        if let Some(remote) = sub_repo.remote_default_name(direction) {
            return Ok(remote.to_str_lossy().into_owned());
        }
    }
    let remote_names = sub_repo.remote_names();
    if remote_names.iter().any(|name| name.as_bstr() == b"origin") {
        return Ok("origin".to_owned());
    }
    if remote_names.len() == 1
        && let Some(only) = remote_names.into_iter().next()
    {
        return Ok(only.to_str_lossy().into_owned());
    }
    anyhow::bail!("failed to determine push remote for submodule")
}

/// Inspect submodules whose gitlink (`EntryKind::Commit`) changed between the workspace base
/// commit and `gitbutler/workspace` (or `HEAD`) and return those that are not yet synced with
/// their submodule remote.
pub fn check_out_of_sync_submodules(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
) -> Result<Vec<OutOfSyncBaseTarget>> {
    let Some(workdir) = repo.workdir() else {
        return Ok(Vec::new());
    };
    let base_sha = project_meta.resolve_target_commit(repo).ok();
    let workspace_sha = repo
        .try_find_reference(WORKSPACE_REF_NAME)?
        .map(|mut r| r.peel_to_commit().map(|c| c.id))
        .transpose()?
        .or_else(|| repo.head_id().ok().map(|id| id.detach()));
    let Some(workspace_sha) = workspace_sha else {
        return Ok(Vec::new());
    };
    if base_sha == Some(workspace_sha) {
        return Ok(Vec::new());
    }

    let changes = but_core::diff::tree_changes(repo, base_sha, workspace_sha)?;
    let mut out = Vec::new();
    for change in changes {
        let Some(state) = change.status.state() else {
            continue;
        };
        if state.kind != gix::object::tree::EntryKind::Commit {
            continue;
        }
        let sub_rel_path = change.path.to_str_lossy().into_owned();
        let sub_workdir = workdir.join(&sub_rel_path);
        let Ok(sub_repo) = gix::open(&sub_workdir) else {
            continue;
        };
        let Ok(remote_name) = resolve_submodule_push_remote(&sub_repo) else {
            continue;
        };
        let gitlink_oid = state.id;

        // 1. Determine target branch if configured or matching local HEAD / branch
        let mut branch_short = read_gitmodules_branch(workdir, &sub_rel_path);
        if branch_short.is_none()
            && let Ok(head) = sub_repo.head()
            && let Some(referent) = head.referent_name()
            && referent.category() == Some(Category::LocalBranch)
        {
            branch_short = Some(referent.shorten().to_str_lossy().into_owned());
        }
        if branch_short.is_none()
            && let Ok(platform) = sub_repo.references()
            && let Ok(iter) = platform.prefixed("refs/heads/")
        {
            for reference in iter.flatten() {
                let short = reference.name().shorten().to_str_lossy().into_owned();
                let mut reference = reference;
                if reference.peel_to_commit().is_ok_and(|c| c.id == gitlink_oid) {
                    branch_short = Some(short);
                    break;
                }
            }
        }

        if let Some(short_name) = branch_short {
            let tracking_ref = format!("refs/remotes/{remote_name}/{short_name}");
            let remote_oid = sub_repo
                .try_find_reference(tracking_ref.as_str())?
                .and_then(|mut r| r.peel_to_commit().ok().map(|c| c.id));
            if remote_oid == Some(gitlink_oid) {
                continue;
            }
            out.push(OutOfSyncBaseTarget {
                path: Some(sub_rel_path),
                ref_name: format!("refs/heads/{short_name}"),
                short_name,
                remote_name,
                is_tag: false,
                target_sha: gitlink_oid.to_string(),
                remote_sha: remote_oid.map(|oid| oid.to_string()),
            });
            continue;
        }

        // 2. Check if gitlink_oid matches a local tag in the submodule
        let mut tag_match: Option<(String, String)> = None;
        if let Ok(platform) = sub_repo.references()
            && let Ok(iter) = platform.prefixed("refs/tags/")
        {
            for reference in iter.flatten() {
                let full_name = reference.name().as_bstr().to_str_lossy().into_owned();
                let short_name = reference.name().shorten().to_str_lossy().into_owned();
                let mut reference = reference;
                if reference.peel_to_commit().is_ok_and(|c| c.id == gitlink_oid) {
                    tag_match = Some((full_name, short_name));
                    break;
                }
            }
        }

        if let Some((ref_name, short_name)) = tag_match {
            let remote_oid = ls_remote_commit_with_askpass(
                &sub_repo,
                &remote_name,
                &ref_name,
                Some("modal".to_owned()),
            )?;
            if remote_oid == Some(gitlink_oid) {
                continue;
            }
            out.push(OutOfSyncBaseTarget {
                path: Some(sub_rel_path),
                ref_name,
                short_name,
                remote_name,
                is_tag: true,
                target_sha: gitlink_oid.to_string(),
                remote_sha: remote_oid.map(|oid| oid.to_string()),
            });
            continue;
        }

        // 3. If any remote tracking branch already points to gitlink_oid, nothing to push
        let mut already_on_remote = false;
        let remote_prefix = format!("refs/remotes/{remote_name}/");
        if let Ok(platform) = sub_repo.references()
            && let Ok(iter) = platform.prefixed(remote_prefix.as_str())
        {
            for mut reference in iter.flatten() {
                if reference.peel_to_commit().is_ok_and(|c| c.id == gitlink_oid) {
                    already_on_remote = true;
                    break;
                }
            }
        }
        if !already_on_remote {
            anyhow::bail!(
                "Submodule `{sub_rel_path}` is at detached commit {gitlink_oid} with no configured branch in `.gitmodules`, local branch, or tag to push to"
            );
        }
    }

    Ok(out)
}

fn push_out_of_sync_submodules(
    repo: &gix::Repository,
    out_of_sync_submodules: &[OutOfSyncBaseTarget],
    cred_config_args: &[String],
) -> Result<()> {
    let Some(workdir) = repo.workdir() else {
        return Ok(());
    };
    for sub in out_of_sync_submodules {
        let Some(sub_path) = sub.path.as_deref() else {
            continue;
        };
        let sub_workdir = workdir.join(sub_path);
        let sub_repo = gix::open(&sub_workdir)
            .with_context(|| format!("failed to open submodule repository at `{sub_path}`"))?;
        let refspec = format!("+{}:{}", sub.target_sha, sub.ref_name);
        push_refspecs_with_askpass(
            &sub_repo,
            &sub.remote_name,
            vec![refspec],
            false,
            false,
            Some(None),
            cred_config_args.to_vec(),
        )
        .with_context(|| {
            format!(
                "failed to push submodule `{sub_path}` ({}) to `{}`",
                sub.ref_name, sub.remote_name
            )
        })?;
    }
    Ok(())
}

/// Fill HTTP(S) credentials once via `git credential fill` for `remote_url` and return `-c`
/// arguments (`-c credential.helper= -c credential.helper=!f() { ... }; f`) so multiple `git push`
/// calls (submodule + superproject) reuse the same credential without prompting twice.
fn fill_shared_http_credential_args(repo: &gix::Repository, remote_url: &str) -> Vec<String> {
    if !(remote_url.starts_with("https://") || remote_url.starts_with("http://")) {
        return Vec::new();
    }
    let git_exe = gix::path::env::exe_invocation();
    let mut cmd = std::process::Command::new(git_exe);
    cmd.current_dir(repo.git_dir())
        .args(["credential", "fill"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(mut child) = cmd.spawn() else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "url={remote_url}");
        let _ = writeln!(stdin);
    }
    let Ok(output) = child.wait_with_output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut username = None;
    let mut password = None;
    for line in stdout.lines() {
        if let Some(u) = line.strip_prefix("username=") {
            username = Some(u.trim_end_matches('\r').to_owned());
        } else if let Some(p) = line.strip_prefix("password=") {
            password = Some(p.trim_end_matches('\r').to_owned());
        }
    }
    let (Some(user), Some(pass)) = (username, password) else {
        return Vec::new();
    };
    // Escape single quotes for POSIX sh inside `!f() { ... }; f`
    let esc_user = user.replace('\'', "'\\''");
    let esc_pass = pass.replace('\'', "'\\''");
    let helper = format!(
        "!f() {{ printf '%s\\n' 'username={esc_user}' 'password={esc_pass}'; }}; f"
    );
    vec![
        "-c".to_owned(),
        "credential.helper=".to_owned(),
        "-c".to_owned(),
        format!("credential.helper={helper}"),
    ]
}

/// Check whether the workspace base target (`project_meta.target_ref` / `target_commit_id`) is out
/// of sync with its remote.
///
/// - For a tag target (`refs/tags/<tag>`), queries `push_remote` via `git ls-remote` to check if
///   the remote tag exists and peels to `target_sha`.
/// - For a branch target (`refs/heads/<branch>`), compares `target_sha` against the remote-tracking
///   ref (`refs/remotes/<remote>/<branch>`).
pub fn check_out_of_sync_base_target(
    repo: &gix::Repository,
    project_meta: &ProjectMeta,
    push_remote: &str,
) -> Result<Option<OutOfSyncBaseTarget>> {
    let Some(target_ref) = project_meta.target_ref.as_ref() else {
        return Ok(None);
    };
    let Ok(target_sha) = project_meta.resolve_target_commit(repo) else {
        return Ok(None);
    };

    let ref_name = target_ref.to_string();
    let short_name = target_ref.shorten().to_str_lossy().into_owned();

    if target_ref.is_tag() {
        let remote_oid =
            ls_remote_commit_with_askpass(repo, push_remote, &ref_name, Some("modal".to_owned()))?;
        if remote_oid == Some(target_sha) {
            return Ok(None);
        }
        return Ok(Some(OutOfSyncBaseTarget {
            path: None,
            ref_name,
            short_name,
            remote_name: push_remote.to_owned(),
            is_tag: true,
            target_sha: target_sha.to_string(),
            remote_sha: remote_oid.map(|oid| oid.to_string()),
        }));
    }

    let remote_names = repo.remote_names();
    let (remote_name, remote_branch_short, remote_tracking_ref) =
        if let Some(upstream_ref) = project_meta.remote_target_ref(repo)? {
            let (remote, branch) =
                extract_remote_name_and_short_name(upstream_ref.as_ref(), &remote_names)
                    .unwrap_or_else(|| (push_remote.to_owned(), short_name.as_str().into()));
            (remote, branch.to_str_lossy().into_owned(), upstream_ref)
        } else {
            let tracking_ref: gix::refs::FullName =
                format!("refs/remotes/{push_remote}/{short_name}").try_into()?;
            (push_remote.to_owned(), short_name, tracking_ref)
        };

    let remote_oid = repo
        .try_find_reference(remote_tracking_ref.as_ref())?
        .map(|mut r| r.peel_to_commit().map(|c| c.id))
        .transpose()?;

    if remote_oid == Some(target_sha) {
        return Ok(None);
    }

    Ok(Some(OutOfSyncBaseTarget {
        path: None,
        ref_name: format!("refs/heads/{remote_branch_short}"),
        short_name: remote_branch_short,
        remote_name,
        is_tag: false,
        target_sha: target_sha.to_string(),
        remote_sha: remote_oid.map(|oid| oid.to_string()),
    }))
}

/// Push `gitbutler/workspace` to `workspace_remote_branch` on the default remote (persisting its
/// tracking configuration in local Git config), along with the workspace base if it is not synced
/// with upstream and all applied branches that are out of sync.
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
    let remote_url_str = remote_url.to_bstring().to_str_lossy().into_owned();

    // Check and push any out-of-sync submodules whose gitlinks were modified in the workspace.
    // When at least one submodule also needs to be pushed, fill the HTTP(S) credential once so
    // both the submodule push and the superproject push reuse it without prompting twice.
    let out_of_sync_submodules = check_out_of_sync_submodules(repo, project_meta)?;
    let shared_cred_args = if out_of_sync_submodules.is_empty() {
        Vec::new()
    } else {
        fill_shared_http_credential_args(repo, &remote_url_str)
    };
    push_out_of_sync_submodules(repo, &out_of_sync_submodules, &shared_cred_args)?;

    if run_hooks {
        match hooks::pre_push(
            repo,
            &push_remote,
            &remote_url_str,
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

        if let Some(base) = check_out_of_sync_base_target(repo, project_meta, &push_remote)? {
            let base_sha = gix::ObjectId::from_hex(base.target_sha.as_bytes())?;
            let before_sha = base
                .remote_sha
                .clone()
                .unwrap_or_else(|| repo.object_hash().null().to_string());
            let (remote_tracking_ref, refspec) = if base.is_tag {
                let tag_ref: gix::refs::FullName = base.ref_name.clone().try_into()?;
                let refspec = format!("+{}:{}", base.target_sha, base.ref_name);
                (tag_ref, refspec)
            } else {
                let remote_ref: gix::refs::FullName =
                    format!("refs/remotes/{}/{}", base.remote_name, base.short_name).try_into()?;
                let refspec = format!("+{}:{}", base.target_sha, base.ref_name);
                (remote_ref, refspec)
            };
            push_refspecs_with_askpass(
                repo,
                &base.remote_name,
                vec![refspec],
                false,
                false,
                Some(None),
                vec![],
            )?;
            let local_label = if base.is_tag {
                base.ref_name
            } else {
                base.short_name.clone()
            };
            result.branch_to_remote.push((
                local_label.clone(),
                remote_tracking_ref,
                base.short_name,
            ));
            result
                .branch_sha_updates
                .push((local_label, before_sha, base_sha.to_string()));
        }

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

    if let Some(base) = check_out_of_sync_base_target(repo, project_meta, &push_remote)? {
        let before_sha = base
            .remote_sha
            .clone()
            .unwrap_or_else(|| repo.object_hash().null().to_string());
        let (remote_tracking_ref, local_label) = if base.is_tag {
            let tag_ref: gix::refs::FullName = base.ref_name.clone().try_into()?;
            (tag_ref, base.ref_name.clone())
        } else {
            let remote_ref: gix::refs::FullName =
                format!("refs/remotes/{}/{}", base.remote_name, base.short_name).try_into()?;
            (remote_ref, base.short_name.clone())
        };
        let refspec = format!("+{}:{}", base.target_sha, base.ref_name);
        if let Some((_, list)) = refspecs_by_remote
            .iter_mut()
            .find(|(r, _)| r == &base.remote_name)
        {
            list.push(refspec);
        } else {
            refspecs_by_remote.push((base.remote_name.clone(), vec![refspec]));
        }
        pending_updates.push((
            (local_label.clone(), remote_tracking_ref, base.short_name),
            (local_label, before_sha, base.target_sha),
        ));
    }

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
        push_refspecs_with_askpass(
            repo,
            &remote_name,
            refspecs,
            false,
            false,
            Some(None),
            shared_cred_args.clone(),
        )?;
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
