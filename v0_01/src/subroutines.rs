//! Subroutines: multi-step flows invoked from the REPL with `run <name> [args...]`.
//!
//! Where the single-step commands (`open`, `click`, ...) do one WebDriver
//! call each, a subroutine is a small program of its own — navigate,
//! fill a form, poll the page, wait for a human challenge — that blocks
//! the prompt until it finishes. Subroutines are how the agent acts out
//! a whole task rather than a single instruction.
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

/// How often poll loops re-check the page.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How long form-fill waits for the next login step's field/button.
const STEP_TIMEOUT: Duration = Duration::from_secs(30);

/// How long login confirmation (including human 2FA) may take.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600); // 10 minutes

/// How long to wait for the bookmarks page after navigating there.
const BOOKMARKS_TIMEOUT: Duration = Duration::from_secs(60);

/// Direct x.com login flow — more stable than the marketing landing wall.
const LOGIN_FLOW_URL: &str = "https://x.com/i/flow/login";

/// Bookmarks page the subroutine aims to reach.
const BOOKMARKS_URL: &str = "https://x.com/i/bookmarks";

/// Username / email field on the login flow (x.com uses either name).
const USERNAME_SELECTOR: &str =
    "input[name=\"text\"], input[name=\"username_or_email\"]";

/// Password field on the login flow.
const PASSWORD_SELECTOR: &str = "input[name=\"password\"]";

/// The name → one-line-description table.
///
/// Two consumers: `help` prints every row, and [`check_known`]'s error
/// lists the names. Keeping the table in one place means the help text
/// can never drift from the dispatch in [`execute`].
pub fn registry() -> &'static [(&'static str, &'static str)] {
    &[(
        "twitter-bookmarks",
        "<username> <password> — log in and open bookmarks",
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
///
/// `args` are the tokens after the subroutine name (e.g. username and
/// password). Handlers validate arity themselves so each subroutine can
/// document its own usage.
pub async fn execute(session: &BrowserSession, name: &str, args: &[String]) -> Result<()> {
    // check_known uses String errors so it stays independent of anyhow;
    // wrap them explicitly (anyhow has no From<String> on purpose).
    check_known(name).map_err(anyhow::Error::msg)?;
    match name {
        "twitter-bookmarks" => {
            let (username, password) = match args {
                [u, p, ..] => (u.as_str(), p.as_str()),
                _ => bail!(
                    "usage: run twitter-bookmarks <username> <password>\n\
                     Quote passwords with spaces: run twitter-bookmarks user \"p@ss word\""
                ),
            };
            twitter_bookmarks(session, username, password).await
        }
        // check_known guards this match; a new subroutine gets a new arm.
        _ => unreachable!("check_known already rejected \"{name}\""),
    }
}

/// Log into X/Twitter with the given credentials and open bookmarks.
///
/// Steps:
/// 1. Prefer an already-open Twitter window (attach mode); otherwise stay
///    on the current window.
/// 2. If the page already looks logged in, skip the form.
/// 3. Otherwise open the login flow, fill username → Next → password →
///    Log in. Never prints the password.
/// 4. Poll until login is confirmed. Unusual-activity / 2FA challenges
///    fall through to a human in the browser window (same timeout as
///    before).
/// 5. Navigate to `/i/bookmarks` and wait until that page has loaded.
///
/// Image bookmark extraction is intentionally out of scope here — this
/// subroutine only proves login + navigation.
pub async fn twitter_bookmarks(
    session: &BrowserSession,
    username: &str,
    password: &str,
) -> Result<()> {
    ensure_twitter_context(session).await?;

    if looks_logged_in(session).await? {
        println!("already logged in — skipping the login form.");
    } else {
        println!("opening the X login flow for user \"{username}\"...");
        session.open(LOGIN_FLOW_URL).await?;
        fill_login_form(session, username, password)
            .await
            .with_context(|| {
                "login form fill failed — try `screenshot login-fail.png` to see the page"
            })?;
        println!(
            "credentials submitted — waiting for login \
             (complete any 2FA / unusual-login challenge in the browser; Ctrl+C cancels)."
        );
        wait_for_login(session, LOGIN_TIMEOUT).await?;
        println!("login detected.");
    }

    println!("opening bookmarks...");
    session.open(BOOKMARKS_URL).await?;
    wait_for_bookmarks(session, BOOKMARKS_TIMEOUT).await?;
    println!("bookmarks page ready.");
    Ok(())
}

/// Switch to an open Twitter/X window if one exists; otherwise leave the
/// current window as-is (the caller will navigate).
async fn ensure_twitter_context(session: &BrowserSession) -> Result<()> {
    match find_twitter_window(session).await {
        Ok(Some((handle, url))) => {
            session.switch_to_window(&handle).await?;
            println!("attached to your Twitter window ({url}).");
        }
        Ok(None) => {}
        Err(err) => {
            // Window scanning is best-effort; never block the subroutine
            // on it — fall through and navigate the current window.
            println!("note: could not scan the browser windows ({err:#}); continuing.");
        }
    }
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

/// One-shot probe: true when the current page looks logged in.
///
/// Same markers as [`wait_for_login`], without the two-poll confirmation
/// — good enough to skip a redundant form fill when attach mode already
/// has a session.
async fn looks_logged_in(session: &BrowserSession) -> Result<bool> {
    let logged_out_marker = session
        .eval_bool(LOGGED_OUT_MARKER_SCRIPT)
        .await
        .context("could not check the login state — is the browser tab still open?")?;
    let has_text = session
        .eval_bool(BODY_HAS_TEXT_SCRIPT)
        .await
        .context("could not read the page — is the browser tab still open?")?;
    Ok(!logged_out_marker && has_text)
}

/// Fill the multi-step x.com login form: username → Next → password → Log in.
///
/// Does not print or log the password. Callers should already have
/// navigated to [`LOGIN_FLOW_URL`].
async fn fill_login_form(session: &BrowserSession, username: &str, password: &str) -> Result<()> {
    wait_until_true(
        session,
        &format!("return !!document.querySelector({USERNAME_SELECTOR:?});"),
        STEP_TIMEOUT,
        "the username field",
    )
    .await?;

    session
        .clear(USERNAME_SELECTOR)
        .await
        .context("could not clear the username field")?;
    session
        .type_text(USERNAME_SELECTOR, username)
        .await
        .context("could not type the username")?;
    println!("  typed username — clicking Next...");
    click_role_button(session, "Next").await?;

    wait_until_true(
        session,
        &format!("return !!document.querySelector({PASSWORD_SELECTOR:?});"),
        STEP_TIMEOUT,
        "the password field",
    )
    .await?;

    session
        .clear(PASSWORD_SELECTOR)
        .await
        .context("could not clear the password field")?;
    session
        .type_text(PASSWORD_SELECTOR, password)
        .await
        .context("could not type the password")?;
    println!("  typed password — clicking Log in...");
    // x.com labels the submit button "Log in" (with a space).
    click_role_button(session, "Log in").await?;
    Ok(())
}

/// Click the first `[role="button"]` / `<button>` whose visible text
/// matches `label` exactly (trimmed).
async fn click_role_button(session: &BrowserSession, label: &str) -> Result<()> {
    // Debug-format the label into a JS string literal so quotes/backslashes
    // in the label cannot break out of the script.
    let script = format!(
        "const buttons = Array.from(document.querySelectorAll('[role=\"button\"], button')); \
         const b = buttons.find(el => (el.innerText || '').trim() === {label:?}); \
         if (!b) return false; \
         b.click(); \
         return true;"
    );
    let clicked = session
        .eval_bool(&script)
        .await
        .with_context(|| format!("could not click the \"{label}\" button"))?;
    if !clicked {
        bail!(
            "no \"{label}\" button found — try `screenshot login-fail.png` to see the page"
        );
    }
    Ok(())
}

/// Poll until `script` returns true, or `timeout` expires.
async fn wait_until_true(
    session: &BrowserSession,
    script: &str,
    timeout: Duration,
    what: &str,
) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        if session
            .eval_bool(script)
            .await
            .with_context(|| format!("could not check for {what}"))?
        {
            return Ok(());
        }
        if start.elapsed() >= timeout {
            bail!(
                "timed out waiting for {what} after {}s — \
                 try `screenshot login-fail.png` to see the page",
                timeout.as_secs()
            );
        }
        sleep(POLL_INTERVAL).await;
    }
}

/// JavaScript probe: true while the page still shows the logged-out
/// login wall / flow.
///
/// Verified against the live x.com landing (2026-11-08) and the login
/// flow fields used by [`fill_login_form`]: username inputs, password
/// input, Google sign-in iframe, and the mobile Sign-in link. None of
/// these exist after a successful login. Several independent markers
/// reduce the chance of missing a wall variant when x.com changes its
/// markup again.
const LOGGED_OUT_MARKER_SCRIPT: &str = "return !!document.querySelector(\
    'input[name=\"username_or_email\"], \
    input[name=\"text\"][autocomplete=\"username\"], \
    input[name=\"password\"], \
    a[href^=\"/i/jf/onboarding/web\"], \
    iframe[title=\"Sign in with Google Button\"]');";

/// JavaScript probe: true when the page body has visible text. Excludes
/// empty intermediate pages during redirects.
const BODY_HAS_TEXT_SCRIPT: &str =
    "const t = document.body ? document.body.innerText.trim() : ''; return t.length > 0;";

/// Poll the current page until it looks logged in, or `timeout` expires.
///
/// A poll "looks logged out" when [`LOGGED_OUT_MARKER_SCRIPT`] finds a
/// login affordance. Login is declared after two consecutive polls that
/// both (a) show a page with actual text and (b) show no logged-out
/// markers. The two-poll rule absorbs redirects and loading windows;
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
                 complete any challenge in the browser, then run again",
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

/// Poll until the current URL is the bookmarks page and the body has text.
pub async fn wait_for_bookmarks(session: &BrowserSession, timeout: Duration) -> Result<()> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            bail!(
                "timed out waiting for the bookmarks page after {}s — \
                 try `screenshot login-fail.png` (logged out?) or check the URL",
                timeout.as_secs()
            );
        }

        let url = session.current_url().await?;
        let on_bookmarks = url.contains("/i/bookmarks");
        let has_text = session
            .eval_bool(BODY_HAS_TEXT_SCRIPT)
            .await
            .context("could not read the bookmarks page")?;

        if on_bookmarks && has_text {
            // If we landed on bookmarks but the login wall is still
            // showing, the session isn't authenticated — fail loudly.
            let logged_out = session
                .eval_bool(LOGGED_OUT_MARKER_SCRIPT)
                .await
                .context("could not check login state on the bookmarks page")?;
            if logged_out {
                bail!(
                    "reached a bookmarks URL but the login wall is still visible — \
                     login did not stick; try attach mode or `screenshot login-fail.png`"
                );
            }
            return Ok(());
        }

        if start.elapsed().as_secs().is_multiple_of(10) {
            println!(
                "  waiting for bookmarks... ({}s elapsed, url={url})",
                start.elapsed().as_secs()
            );
        }
        sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_lists_twitter_bookmarks() {
        assert!(registry().contains(&(
            "twitter-bookmarks",
            "<username> <password> — log in and open bookmarks"
        )));
        assert!(!registry().is_empty());
    }

    #[test]
    fn check_known_accepts_registered_names() {
        assert!(check_known("twitter-bookmarks").is_ok());
    }

    #[test]
    fn check_known_rejects_unknown_names_with_the_list() {
        let err = check_known("nope").unwrap_err();
        assert!(err.contains("unknown subroutine \"nope\""));
        assert!(err.contains("twitter-bookmarks"));
    }
}
