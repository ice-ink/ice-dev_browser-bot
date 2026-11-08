//! Live-browser smoke test.
//!
//! This test starts a real Chrome window and drives it through the same
//! [`BrowserSession`] the REPL uses. It is skipped (with a notice) when no
//! browser can be started — e.g. no Chrome installed, or the chromedriver
//! download is blocked — so `cargo test` still passes on such machines.
//!
//! The page under test is a `data:` URL, which needs no network and no
//! server: Chrome renders the HTML directly from the URL itself. (Spaces
//! are percent-encoded as %20 because URLs can't contain raw spaces.)

use browser_bot::browser::BrowserSession;

#[tokio::test]
async fn drives_a_real_browser() {
    let session = match BrowserSession::connect().await {
        Ok(session) => session,
        Err(err) => {
            eprintln!("SKIPPED: could not start a browser session ({err:#})");
            return;
        }
    };

    // A tiny page: a heading, a button whose click handler writes to a
    // paragraph, and a text field.
    let page = "data:text/html,<html><body>\
        <h1 id=\"title\">hello%20bot</h1>\
        <button id=\"btn\" onclick=\"document.getElementById('out').textContent='clicked'\">push</button>\
        <p id=\"out\"></p>\
        <input id=\"field\">\
        </body></html>";

    session.open(page).await.expect("open the test page");

    // read: the whole page's visible text contains the heading.
    let page_text = session.read_page().await.expect("read page text");
    assert!(page_text.contains("hello bot"), "page text was: {page_text}");

    // click: the button's handler sets the paragraph text.
    session.click("#btn").await.expect("click the button");
    let out = session.read_element("#out").await.expect("read #out");
    assert_eq!(out, "clicked");

    // type: send keys into the field, then read the value property.
    // (Element text is empty for inputs, so we use the `value` property.)
    session.type_text("#field", "typed!").await.expect("type into the field");
    let value = session.prop("#field", "value").await.expect("read value property");
    assert_eq!(value.as_deref(), Some("typed!"));

    // screenshot: a PNG file must be created and non-empty.
    let shot = std::env::temp_dir().join("browser_bot_integration_test.png");
    let shot = shot.to_str().expect("temp dir is valid UTF-8");
    session.screenshot(shot).await.expect("take a screenshot");
    let bytes = std::fs::metadata(shot).expect("screenshot file exists").len();
    assert!(bytes > 0, "screenshot file is empty");
    std::fs::remove_file(shot).ok(); // tidy up; ignore errors

    // The test ends by quitting explicitly — same rule as the REPL.
    session.quit().await.expect("close the browser");
}
