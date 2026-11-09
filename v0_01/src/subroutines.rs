//! Subroutines: multi-step flows invoked from the REPL with `run <name>`.
//!
//! Where the single-step commands (`open`, `click`, ...) do one WebDriver
//! call each, a subroutine is a small program of its own — navigate,
//! poll the page, wait for a human — that blocks the prompt until it
//! finishes. Subroutines are how the agent acts out a whole task rather
//! than a single instruction.
//!
//! Structure (kept deliberately flat):
//! * [`registry`] — the name → description table used by `help` and by
//!   error messages.
//! * [`check_known`] — pure name validation, unit-testable without a
//!   browser session.
//! * [`execute`] — dispatch: the one place that maps names to handlers.
//! * one `pub async fn` per subroutine, below.
//!
//! Adding a subroutine = add one handler + one registry row. Nothing
//! else in the crate needs to know it exists.

use std::time::Duration;

use anyhow::{bail, Context, Result};
use thirtyfour::WindowHandle;
use tokio::time::sleep;

use crate::browser::BrowserSession;

/// How often [`wait_for_login`] re-checks the page.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long `twitter-login` gives a human to complete the login.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600); // 10 minutes

/// The name → one-line-description table.
///
/// Two consumers: `help` prints every row, and [`check_known`]'s error
/// lists the names. Keeping the table in one place means the help text
/// can never drift from the dispatch in [`execute`].
pub fn registry() -> &'static [(&'static str, &'static str)] {
    &[(
        "twitter-login",
        "open Twitter (your Chrome if attached) and wait for the user to log in",
    )]
}

/// Pure existence check, separated from [`execute`] so it can be
/// unit-tested without a browser session.
pub fn check_known(name: &str) -> Result<(), String> {
    if registry().iter().any(|(n, _)| *n == name) {
        Ok(())
    } else {
        let available = registry()
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(", ");
        Err(format!(
            "unknown subroutine \"{name}\" — available: {available}"
        ))
    }
}

/// Run a named subroutine. This is the single dispatch point; while it
/// awaits, the REPL does not read any further instructions.
pub async fn execute(session: &BrowserSession, name: &str) -> Result<()> {
    // check_known uses String errors so it stays independent of anyhow;
    // wrap them explicitly (anyhow has no From<String> on purpose).
    check_known(name).map_err(anyhow::Error::msg)?;
    match name {
        "twitter-login" => twitter_login(session).await,
        // check_known guards this match; a new subroutine gets a new arm.
        _ => unreachable!("check_known already rejected \"{name}\""),
    }
}

/// Open Twitter and block until the user has logged in.
///
/// Steps:
/// 1. If one of the open windows is already on Twitter (twitter.com /
///    x.com) — typically the user's own Chrome in attach mode — switch
///    to that window instead of navigating, so an existing login is
///    picked up. Otherwise navigate the current window to
///    <https://twitter.com> (Twitter redirects to x.com).
/// 2. Poll the page until login is detected, up to [`LOGIN_TIMEOUT`].
///
/// The user logs in by hand in the browser window; this code never sees
/// credentials. While this runs, the REPL processes no other
/// instructions. Ctrl+C cancels just this subroutine (the REPL races it
/// against the signal).
pub async fn twitter_login(session: &BrowserSession) -> Result<()> {
    match find_twitter_window(session).await {
        Ok(Some((handle, url))) => {
            session.switch_to_window(&handle).await?;
            println!("attached to your Twitter window ({url}).");
            wait_for_login(session, LOGIN_TIMEOUT).await?;
            println!("login confirmed — driving your window.");
            return Ok(());
        }
        Ok(None) => {}
        Err(err) => {
            // Window scanning is best-effort; never block the subroutine
            // on it — fall back to navigating the current window.
            println!("note: could not scan the browser windows ({err:#}); navigating instead.");
        }
    }

    println!(
        "opening Twitter — log in in the browser window. \
         The prompt waits for you (Ctrl+C cancels)."
    );
    session.open("https://twitter.com").await?;
    wait_for_login(session, LOGIN_TIMEOUT).await?;
    println!("login detected — continuing.");
    Ok(())
}

/// Find a window whose URL is on Twitter, if any.
///
/// Returns the handle *and* URL. The URL is the display value; the
/// caller switches to the handle (enumeration restores the original
/// window, so switching is a separate explicit step).
///
/// The user's Twitter tab may still be committing its navigation when
/// the subroutine starts, so the scan retries a few times before
/// concluding there is no Twitter window.
async fn find_twitter_window(session: &BrowserSession) -> Result<Option<(WindowHandle, String)>> {
    println!("looking for an open Twitter window...");
    for _ in 0..5 {
        for (handle, url) in session.windows_with_urls().await? {
            if url.contains("twitter.com") || url.contains("x.com") {
                return Ok(Some((handle, url)));
            }
        }
        sleep(POLL_INTERVAL).await;
    }
    Ok(None)
}

/// JavaScript probe: true while the page still shows the logged-out
/// login wall.
///
/// Verified against the live x.com landing (2026-11-08): the current
/// wall uses a "jetfuel" auth form — `input[name="username_or_email"]`
/// plus a hidden `input[name="password"]` (which stays in the DOM on the
/// password step too), a Google sign-in button iframe, and a mobile
/// "Sign in" link into /i/jf/onboarding/web. None of these exist after
/// login. Several independent markers reduce the chance of missing a
/// wall variant when x.com changes its markup again.
const LOGGED_OUT_MARKER_SCRIPT: &str = "return !!document.querySelector(\
    'input[name=\"username_or_email\"], \
    input[name=\"password\"], \
    a[href^=\"/i/jf/onboarding/web\"], \
    iframe[title=\"Sign in with Google Button\"]');";

/// JavaScript probe: true when the page body has visible text. Excludes
/// empty intermediate pages during the twitter.com → x.com redirect.
const BODY_HAS_TEXT_SCRIPT: &str =
    "const t = document.body ? document.body.innerText.trim() : ''; return t.length > 0;";

/// Poll the current page until it looks logged in, or `timeout` expires.
///
/// A poll "looks logged out" when [`LOGGED_OUT_MARKER_SCRIPT`] finds a
/// login affordance. Login is declared after two consecutive polls that
/// both (a) show a page with actual text and (b) show no logged-out
/// markers. The two-poll rule absorbs the redirect and loading windows;
/// the text rule stops an empty intermediate page from counting as a
/// false positive.
///
/// Deliberately parameterized — the caller navigates, and the timeout is
/// an argument — so integration tests can run it against a local `data:`
/// URL with a short timeout. No Twitter account required.
pub async fn wait_for_login(session: &BrowserSession, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    let mut clean_polls = 0u32;

    loop {
        if start.elapsed() >= timeout {
            bail!(
                "timed out waiting for login after {}s — \
                 run `run twitter-login` again to retry",
                timeout.as_secs()
            );
        }

        // Either probe failing means something is wrong with the page or
        // the tab (closed, navigated away); surface it instead of
        // looping forever.
        let logged_out_marker = session
            .eval_bool(LOGGED_OUT_MARKER_SCRIPT)
            .await
            .context("could not check the login state — is the browser tab still open?")?;
        let has_text = session
            .eval_bool(BODY_HAS_TEXT_SCRIPT)
            .await
            .context("could not read the page — is the browser tab still open?")?;

        if !logged_out_marker && has_text {
            clean_polls += 1;
            if clean_polls >= 2 {
                return Ok(());
            }
            println!(
                "  no login prompt visible ({}s elapsed) — confirming on the next check...",
                start.elapsed().as_secs()
            );
        } else {
            clean_polls = 0;
            // Status every 10 seconds: enough to show life without
            // spamming the terminal during a 10-minute login.
            if start.elapsed().as_secs().is_multiple_of(10) {
                println!(
                    "  waiting for login... ({}s elapsed, checking every {}s)",
                    start.elapsed().as_secs(),
                    POLL_INTERVAL.as_secs()
                );
            }
        }

        sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_twitter_login() {
        assert!(registry().contains(&(
            "twitter-login",
            "open Twitter (your Chrome if attached) and wait for the user to log in"
        )));
        assert!(!registry().is_empty());
    }

    #[test]
    fn check_known_accepts_registered_names() {
        assert!(check_known("twitter-login").is_ok());
    }

    #[test]
    fn check_known_rejects_unknown_names_with_the_list() {
        let err = check_known("nope").unwrap_err();
        assert!(err.contains("unknown subroutine \"nope\""));
        assert!(err.contains("twitter-login"));
    }
}
