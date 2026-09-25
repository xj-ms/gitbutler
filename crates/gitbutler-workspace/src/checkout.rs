//! Forced `git2` checkouts that don't abort when a directory can't be removed.
//!
//! When libgit2 deletes a file during checkout, it also tries to `rmdir` each parent directory and
//! relies on the OS refusing to remove non-empty ones. On Windows, a directory that another process
//! holds open (a shell, an IDE, a build server) fails with a sharing violation *before* emptiness is
//! checked, which libgit2 treats as fatal, aborting the checkout midway even though the directory
//! would have stayed anyway. `GIT_CHECKOUT_SKIP_LOCKED_DIRECTORIES` makes libgit2 skip such
//! directories instead, but `git2` doesn't expose it, so these functions call libgit2 directly.
//!
//! TODO: use `CheckoutBuilder` directly once `git2` offers a setter for this flag.

use git2::{Binding as _, build::CheckoutBuilder};
use libgit2_sys as raw;

/// Like [`git2::Repository::checkout_index()`], but skips directories that can't be removed.
pub fn checkout_index(
    repo: &git2::Repository,
    index: Option<&mut git2::Index>,
    opts: &mut CheckoutBuilder<'_>,
) -> Result<(), git2::Error> {
    let index = index.map_or(std::ptr::null_mut(), |index| index.raw());
    // SAFETY: `repo` and `index` are valid for the duration of the call, and `raw_opts` only
    // borrows data owned by `opts`, which outlives it.
    checkout(opts, |raw_opts| unsafe {
        raw::git_checkout_index(repo.raw(), index, raw_opts)
    })
}

/// Like [`git2::Repository::checkout_tree()`], but skips directories that can't be removed.
pub fn checkout_tree(
    repo: &git2::Repository,
    treeish: &git2::Object<'_>,
    opts: &mut CheckoutBuilder<'_>,
) -> Result<(), git2::Error> {
    // SAFETY: see `checkout_index()`.
    checkout(opts, |raw_opts| unsafe {
        raw::git_checkout_tree(repo.raw(), treeish.raw(), raw_opts)
    })
}

/// Like [`git2::Repository::checkout_head()`], but skips directories that can't be removed.
pub fn checkout_head(
    repo: &git2::Repository,
    opts: &mut CheckoutBuilder<'_>,
) -> Result<(), git2::Error> {
    // SAFETY: see `checkout_index()`.
    checkout(opts, |raw_opts| unsafe {
        raw::git_checkout_head(repo.raw(), raw_opts)
    })
}

fn checkout(
    opts: &mut CheckoutBuilder<'_>,
    run: impl FnOnce(&raw::git_checkout_options) -> std::os::raw::c_int,
) -> Result<(), git2::Error> {
    // SAFETY: this mirrors what `git2` does for its own checkout functions; `git_checkout_options`
    // is a plain C struct that `git_checkout_init_options()` fully initializes.
    let mut raw_opts: raw::git_checkout_options = unsafe { std::mem::zeroed() };
    check(unsafe {
        raw::git_checkout_init_options(&mut raw_opts, raw::GIT_CHECKOUT_OPTIONS_VERSION)
    })?;
    // SAFETY: `raw_opts` is initialized and doesn't outlive `opts`.
    unsafe { opts.configure(&mut raw_opts) };
    raw_opts.checkout_strategy |= raw::GIT_CHECKOUT_SKIP_LOCKED_DIRECTORIES as u32;
    check(run(&raw_opts))
}

fn check(code: std::os::raw::c_int) -> Result<(), git2::Error> {
    if code < 0 {
        Err(git2::Error::last_error(code))
    } else {
        Ok(())
    }
}
