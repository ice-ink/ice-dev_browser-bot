//! Window-plumbing tests against a real browser.
//!
//! Attach mode (driving the user's own Chrome) relies on enumerating
//! windows, reading their URLs, and switching between them. Those
//! primitives are tested here in normal mode with two tabs — no debug
//! Chrome needed. Skipped with a notice when no browser can start,
//! like the other integration tests.

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

#[tokio::test]
async fn enumerates_and_switches_between_windows() {
    let Some(session) = connect_or_skip().await else { return };

    // Two tabs on distinct data: URLs; each URL embeds its marker word
    // ("one"/"two"), which also lets us recognize them by current_url.
    let before = session.window_handles().await.expect("initial handles");
    session
        .open("data:text/html,<body>one</body>")
        .await
        .expect("open tab one");

    session.new_tab().await.expect("open a second tab");
    // chromedriver does NOT make the new tab current, so switch to it
    // explicitly before navigating. This is the same assumption-free
    // pattern the attach-aware subroutine uses.
    let after = session.window_handles().await.expect("handles after new tab");
    let new_handle = after
        .iter()
        .find(|h| !before.contains(h))
        .expect("exactly one new handle appears");
    session.switch_to_window(new_handle).await.expect("switch to the new tab");
    session
        .open("data:text/html,<body>two</body>")
        .await
        .expect("open tab two");

    // Chromium lists tabs as separate window handles.
    let handles = session.window_handles().await.expect("window handles");
    assert!(handles.len() >= 2, "expected >=2 handles, got {}", handles.len());

    // Enumeration reports each window's URL.
    let listed = session.windows_with_urls().await.expect("enumerate windows");
    assert!(listed.len() >= 2, "expected >=2 entries, got {}", listed.len());
    let urls: Vec<&str> = listed.iter().map(|(_, url)| url.as_str()).collect();
    assert!(
        urls.iter().any(|u| u.contains("one")) && urls.iter().any(|u| u.contains("two")),
        "expected tabs 'one' and 'two' among {urls:?}"
    );

    // Switching to the "two" window makes it the driven window.
    let (two_handle, _) = listed
        .iter()
        .find(|(_, url)| url.contains("two"))
        .expect("the 'two' window is listed");
    session.switch_to_window(two_handle).await.expect("switch to 'two'");
    let current = session.current_url().await.expect("current URL");
    assert!(current.contains("two"), "current URL should be the 'two' tab, got {current}");

    session.quit().await.expect("quit");
}
