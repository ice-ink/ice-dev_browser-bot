//! Subroutine tests against a real browser.
//!
//! Real sites change their markup and need accounts, so these tests run
//! [`browser_bot::subroutines::wait_for_login`] against local `data:`
//! URLs instead: pages that fake the logged-out → logged-in transition
//! with a JS timer. No network, no credentials. Skipped (with a notice)
//! when no browser can be started, exactly like the main integration
//! test — skipping rather than failing keeps `cargo test` green on
//! machines without Chrome.

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
