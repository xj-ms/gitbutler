#[cfg(target_family = "unix")]
use std::time::Duration;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use but_ctx::Context;
use but_project_handle::{ProjectHandle, ProjectHandleOrLegacyProjectId};
#[cfg(target_family = "unix")]
use but_settings::AppSettingsWithDiskSync;
#[cfg(target_family = "unix")]
use but_testsupport::invoke_bash_at_dir;
use gitbutler_watcher::{Change, Handler};
#[cfg(target_family = "unix")]
use gitbutler_watcher::{WatchMode, WatcherHandle};
#[cfg(target_family = "unix")]
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

#[cfg(target_family = "unix")]
const QUIET: Duration = Duration::from_millis(1500);
#[cfg(target_family = "unix")]
const GENEROUS_TIMEOUT_FOR_CI: Duration = Duration::from_secs(10);

#[cfg(target_family = "unix")]
struct Watched {
    project_id: ProjectHandleOrLegacyProjectId,
    app_settings: AppSettingsWithDiskSync,
    changes: UnboundedReceiver<Change>,
    watcher: WatcherHandle,
}

#[cfg(target_family = "unix")]
impl Watched {
    fn ctx(&self) -> anyhow::Result<Context> {
        let mut ctx: Context = self.project_id.clone().try_into()?;
        ctx.settings = self.app_settings.get()?.clone();
        Ok(ctx)
    }

    async fn change_before_quiet(&mut self) -> anyhow::Result<Option<Change>> {
        self.watcher.flush()?;
        Ok(tokio::time::timeout(QUIET, self.changes.recv())
            .await
            .ok()
            .flatten())
    }

    async fn settle(&mut self) -> anyhow::Result<()> {
        while self.change_before_quiet().await?.is_some() {}
        Ok(())
    }

    async fn expect(&mut self, matches: impl Fn(&Change) -> bool) -> anyhow::Result<()> {
        self.watcher.flush()?;
        let matching_change = async {
            while let Some(change) = self.changes.recv().await {
                if matches(&change) {
                    return;
                }
            }
        };
        tokio::time::timeout(GENEROUS_TIMEOUT_FOR_CI, matching_change)
            .await
            .map_err(|_| anyhow::anyhow!("timeout waiting for matching change"))
    }
}

#[cfg(target_family = "unix")]
fn isolated(test: impl Future<Output = anyhow::Result<()>>) -> anyhow::Result<()> {
    but_testsupport::isolated_app_data_dir(|| tokio::runtime::Runtime::new()?.block_on(test))
}

#[cfg(target_family = "unix")]
fn watch(main: &Path, config_dir: &Path) -> anyhow::Result<Watched> {
    let app_settings = AppSettingsWithDiskSync::new_with_customization(config_dir, None)?;
    let project_id = ProjectHandleOrLegacyProjectId::ProjectHandle(ProjectHandle::from_path(main)?);
    let (tx, changes) = unbounded_channel();
    let watcher = gitbutler_watcher::watch_in_background(
        Handler::new(move |change| {
            tx.send(change).ok();
            Ok(())
        }),
        main,
        project_id.clone(),
        app_settings.clone(),
        WatchMode::Legacy,
    )?;
    Ok(Watched {
        project_id,
        app_settings,
        changes,
        watcher,
    })
}

#[cfg(target_family = "unix")]
#[test]
fn linked_worktree_head_moves_are_workspace_activity() -> anyhow::Result<()> {
    isolated(async {
        let tmp = tempfile::tempdir()?;
        invoke_bash_at_dir(
            "git init main && cd main && git commit --allow-empty -m init && git worktree add ../linked",
            tmp.path(),
        );
        let main = tmp.path().join("main");
        let config_dir = tmp.path().join("config");
        watch(&main, &config_dir)?
            .ctx()?
            .set_worktree_archived("linked".into(), false)?;

        let mut watched = watch(&main, &config_dir)?;
        watched.settle().await?;

        invoke_bash_at_dir("git switch --detach", &tmp.path().join("linked"));
        watched
            .expect(|change| matches!(change, Change::WorkspaceActivity { .. }))
            .await
    })
}

#[cfg(target_family = "unix")]
#[test]
fn linked_worktrees_added_while_watching_are_watched() -> anyhow::Result<()> {
    isolated(async {
        let tmp = tempfile::tempdir()?;
        invoke_bash_at_dir(
            "git init main && cd main && git commit --allow-empty -m init",
            tmp.path(),
        );
        let main = tmp.path().join("main");
        let mut watched = watch(&main, &tmp.path().join("config"))?;
        watched.ctx()?.worktrees_with_state()?;

        invoke_bash_at_dir("git worktree add --detach ../linked", &main);
        watched
            .expect(|change| matches!(change, Change::WorkspaceActivity { .. }))
            .await?;
        watched.settle().await?;

        invoke_bash_at_dir(
            "git commit --allow-empty -m detached",
            &tmp.path().join("linked"),
        );
        watched
            .expect(|change| matches!(change, Change::WorkspaceActivity { .. }))
            .await
    })
}

#[cfg(target_family = "unix")]
#[test]
fn linked_worktrees_archived_while_watching_are_not_watched() -> anyhow::Result<()> {
    isolated(async {
        let tmp = tempfile::tempdir()?;
        invoke_bash_at_dir(
            "git init main && cd main && git commit --allow-empty -m init",
            tmp.path(),
        );
        let main = tmp.path().join("main");
        let mut watched = watch(&main, &tmp.path().join("config"))?;
        let ctx = watched.ctx()?;
        ctx.worktrees_with_state()?;

        invoke_bash_at_dir("git worktree add ../linked", &main);
        watched.settle().await?;

        ctx.set_worktree_archived("linked".into(), true)?;
        but_project_handle::write_refresh_sentinel(&ctx.project_data_dir);
        watched.settle().await?;

        invoke_bash_at_dir("git switch --detach", &tmp.path().join("linked"));
        let change = watched.change_before_quiet().await?;
        assert!(
            change.is_none(),
            "nothing is reported about an archived worktree, got {change:?}"
        );
        Ok(())
    })
}

#[cfg(target_family = "unix")]
#[test]
fn linked_worktree_changes_are_reported_for_that_worktree() -> anyhow::Result<()> {
    isolated(async {
        let tmp = tempfile::tempdir()?;
        invoke_bash_at_dir(
            "git init main && cd main && git commit --allow-empty -m init && git worktree add ../linked",
            tmp.path(),
        );
        let main = tmp.path().join("main");
        let linked = tmp.path().join("linked");
        let config_dir = tmp.path().join("config");
        watch(&main, &config_dir)?
            .ctx()?
            .set_worktree_archived("linked".into(), false)?;

        let mut watched = watch(&main, &config_dir)?;
        watched.settle().await?;

        let reports_file = |change: &Change, because_of: &[&str]| match change {
            Change::LinkedWorktreeChanges {
                worktree,
                changes,
                changed_paths,
                ..
            } => {
                worktree == "linked"
                    && changes
                        .worktree_changes
                        .changes
                        .iter()
                        .any(|change| change.path_bytes == "file")
                    // Its hunks come along unassigned, as a linked worktree isn't in the workspace.
                    && changes
                        .assignments
                        .iter()
                        .any(|assignment| assignment.path == "file" && assignment.stack_id.is_none())
                    && changed_paths.iter().eq(because_of.iter().map(Path::new))
            }
            _ => false,
        };

        std::fs::write(linked.join("file"), "content")?;
        watched
            .expect(|change| reports_file(change, &["file"]))
            .await?;
        watched.settle().await?;

        invoke_bash_at_dir("git add file", &linked);
        watched.expect(|change| reports_file(change, &[])).await
    })
}

#[cfg(target_family = "unix")]
#[test]
fn moved_linked_worktrees_are_followed() -> anyhow::Result<()> {
    isolated(async {
        let tmp = tempfile::tempdir()?;
        invoke_bash_at_dir(
            "git init main && cd main && git commit --allow-empty -m init && git worktree add ../linked",
            tmp.path(),
        );
        let main = tmp.path().join("main");
        let config_dir = tmp.path().join("config");
        watch(&main, &config_dir)?
            .ctx()?
            .set_worktree_archived("linked".into(), false)?;

        let mut watched = watch(&main, &config_dir)?;
        watched.settle().await?;

        invoke_bash_at_dir("git worktree move ../linked ../moved", &main);
        watched
            .expect(|change| matches!(change, Change::WorkspaceActivity { .. }))
            .await?;
        watched.settle().await?;

        std::fs::write(tmp.path().join("moved/file"), "content")?;
        watched
            .expect(|change| match change {
                Change::LinkedWorktreeChanges {
                    worktree,
                    changed_paths,
                    ..
                } => worktree == "linked" && changed_paths.iter().eq([Path::new("file")]),
                _ => false,
            })
            .await
    })
}

#[test]
fn git_files_change_handles_platform_path_separators() -> anyhow::Result<()> {
    use but_testsupport::{CommandExt as _, git_at_dir};

    but_testsupport::isolated_app_data_dir(|| {
        let tmp = tempfile::tempdir()?;
        let workdir = tmp.path().join("repo");
        git_at_dir(tmp.path())
            .args(["init", "-b", "main", "repo"])
            .run();
        git_at_dir(&workdir)
            .args(["commit", "--allow-empty", "-m", "init"])
            .run();
        let project_id =
            ProjectHandleOrLegacyProjectId::ProjectHandle(ProjectHandle::from_path(&workdir)?);
        let mut ctx: Context = project_id.clone().try_into()?;
        let mut guard = ctx.exclusive_worktree_access();

        let emitted = Arc::new(Mutex::new(Vec::new()));
        let handler = Handler::new({
            let emitted = Arc::clone(&emitted);
            move |change| {
                emitted.lock().unwrap().push(change);
                Ok(())
            }
        });

        handler.git_files_change(
            project_id,
            vec![
                Path::new("refs").join("heads").join("main"),
                Path::new("refs").join("heads").join("feature"),
                Path::new("logs").join("HEAD"),
            ],
            &mut ctx,
            guard.write_permission(),
        )?;

        let events = emitted.lock().unwrap();
        assert!(
            events
                .iter()
                .filter(|change| matches!(change, Change::GitActivity { .. }))
                .count()
                == 2,
            "HEAD branch ref and logs/HEAD both emit GitActivity even with native path separators"
        );
        assert!(
            events
                .iter()
                .any(|change| matches!(change, Change::WorkspaceActivity { .. })),
            "non-HEAD branch ref emits WorkspaceActivity even with native path separators"
        );
        Ok(())
    })
}
