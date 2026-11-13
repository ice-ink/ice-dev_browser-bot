//! Subroutine tests against a real browser.
//!
//! Real sites change their markup and need accounts, so these tests run
//! against local `data:` URLs instead: pages that fake the logged-out →
//! logged-in transition and a tiny multi-step login form. No network, no
//! real credentials. Skipped (with a notice) when no browser can be
//! started, exactly like the main integration test — skipping rather
//! than failing keeps `cargo test` green on machines without Chrome.

use std::time::Duration;

use browser_bot::browser::BrowserSession;

/// Connect, or print the skip notice. Mirrors the pattern in
/// browser_integration.rs.
async fn connect_or_skip() -> Option<BrowserSession> {
    match BrowserSession::connect().await {
        Ok(session) => Some(session),
        Err(err) => {
            eprintln!("SKIPPED: could not start a browser session ({err:#})");
            None
        }
    }
}

/// Starts logged-out (the login wall's username input is present); a
/// timer removes the input after ~3 seconds, simulating the user
/// finishing a login. (%20 stands for a space — raw spaces don't belong
/// in URLs.)
const LOGIN_THEN_DISAPPEAR: &str = "data:text/html,<body>welcome%20\
    <input name=\"username_or_email\">\
    <script>setTimeout(() => document.querySelector('input').remove(), 3000)</script></body>";

/// Stays logged-out forever: the login input never leaves.
const NEVER_LOGS_IN: &str =
    "data:text/html,<body>welcome%20<input name=\"username_or_email\"></body>";

/// Already logged in: no login button at all, just page text.
const ALREADY_LOGGED_IN: &str = "data:text/html,<body>welcome%20home</body>";

/// Multi-step fake login form: username → Next reveals password → Log in
/// removes the markers (simulates a successful login). Used to exercise
/// form-fill helpers without hitting x.com.
const MULTI_STEP_LOGIN: &str = "data:text/html,<body>\
<div id=\"step\">\
<input name=\"text\" autocomplete=\"username\">\
<div role=\"button\">Next</div>\
</div>\
<script>\
document.querySelector('[role=button]').onclick = () => {\
  document.getElementById('step').innerHTML =\
    '<input name=\"password\" type=\"password\">' +\
    '<div role=\"button\">Log in</div>';\
  document.querySelector('[role=button]').onclick = () => {\
    document.body.innerHTML = 'welcome%20home';\
  };\
};\
</script></body>";

#[tokio::test]
async fn detects_login_when_the_marker_disappears() {
    let Some(session) = connect_or_skip().await else { return };
    session.open(LOGIN_THEN_DISAPPEAR).await.expect("open test page");

    // Marker vanishes after 3s; the two-poll confirmation adds ~4s more.
    // 30s is generous — a broken detector fails the test instead of
    // hanging forever.
    browser_bot::subroutines::wait_for_login(&session, Duration::from_secs(30))
        .await
        .expect("login should be detected after the marker disappears");

    session.quit().await.expect("quit");
}

#[tokio::test]
async fn detects_login_immediately_when_already_logged_in() {
    let Some(session) = connect_or_skip().await else { return };
    session.open(ALREADY_LOGGED_IN).await.expect("open test page");

    browser_bot::subroutines::wait_for_login(&session, Duration::from_secs(30))
        .await
        .expect("an already-logged-in page should pass after confirmation");

    session.quit().await.expect("quit");
}

#[tokio::test]
async fn times_out_when_the_marker_never_leaves() {
    let Some(session) = connect_or_skip().await else { return };
    session.open(NEVER_LOGS_IN).await.expect("open test page");

    let err = browser_bot::subroutines::wait_for_login(&session, Duration::from_secs(6))
        .await
        .expect_err("a page that never logs in should time out");
    let message = format!("{err:#}");
    assert!(message.contains("timed out"), "unexpected error: {message}");

    session.quit().await.expect("quit");
}

#[tokio::test]
async fn wait_for_bookmarks_times_out_off_bookmarks_url() {
    // data: URLs cannot fake a `/i/bookmarks` location (history.replaceState
    // is blocked there), so the positive path is manual acceptance against
    // real x.com. This test locks the timeout / URL-check failure path.
    let Some(session) = connect_or_skip().await else { return };
    session.open(ALREADY_LOGGED_IN).await.expect("open non-bookmarks page");

    let err = browser_bot::subroutines::wait_for_bookmarks(&session, Duration::from_secs(6))
        .await
        .expect_err("a non-bookmarks URL should time out");
    let message = format!("{err:#}");
    assert!(message.contains("timed out"), "unexpected error: {message}");

    session.quit().await.expect("quit");
}

#[tokio::test]
async fn execute_rejects_twitter_bookmarks_without_credentials() {
    let Some(session) = connect_or_skip().await else { return };

    let err = browser_bot::subroutines::execute(&session, "twitter-bookmarks", &[])
        .await
        .expect_err("missing args should fail before touching the page");
    let message = format!("{err:#}");
    assert!(
        message.contains("usage: run twitter-bookmarks"),
        "unexpected error: {message}"
    );

    session.quit().await.expect("quit");
}

#[tokio::test]
async fn multi_step_login_form_reaches_logged_in_state() {
    let Some(session) = connect_or_skip().await else { return };
    session.open(MULTI_STEP_LOGIN).await.expect("open multi-step form");

    // Drive the public execute path against the fake form by navigating
    // first, then calling the same helpers wait_for_login uses after a
    // manual fill via BrowserSession — mirrors fill_login_form without
    // opening the real LOGIN_FLOW_URL.
    session
        .clear("input[name=\"text\"], input[name=\"username_or_email\"]")
        .await
        .ok(); // clear may no-op on a fresh empty field
    session
        .type_text(
            "input[name=\"text\"], input[name=\"username_or_email\"]",
            "alice",
        )
        .await
        .expect("type username");

    // Click Next via the same JS strategy fill_login_form uses.
    let next = session
        .eval_bool(
            "const buttons = Array.from(document.querySelectorAll('[role=\"button\"], button')); \
             const b = buttons.find(el => (el.innerText || '').trim() === 'Next'); \
             if (!b) return false; b.click(); return true;",
        )
        .await
        .expect("click Next");
    assert!(next, "Next button should be clickable");

    // Wait briefly for the password step to render.
    tokio::time::sleep(Duration::from_millis(500)).await;

    session
        .type_text("input[name=\"password\"]", "secret")
        .await
        .expect("type password");
    let login = session
        .eval_bool(
            "const buttons = Array.from(document.querySelectorAll('[role=\"button\"], button')); \
             const b = buttons.find(el => (el.innerText || '').trim() === 'Log in'); \
             if (!b) return false; b.click(); return true;",
        )
        .await
        .expect("click Log in");
    assert!(login, "Log in button should be clickable");

    browser_bot::subroutines::wait_for_login(&session, Duration::from_secs(30))
        .await
        .expect("multi-step fake form should end logged in");

    session.quit().await.expect("quit");
}
