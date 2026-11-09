//! Browser plumbing: a thin wrapper around the `thirtyfour` crate.
//!
//! # The WebDriver pipeline
//!
//! ```text
//! our code ──HTTP──▶ chromedriver ──CDP──▶ Chrome
//! ```
//!
//! [`BrowserSession`] owns a thirtyfour [`WebDriver`] handle. Each method
//! here maps to WebDriver commands ("Navigate To", "Find Element", ...) as
//! defined by the W3C WebDriver specification. We never talk to Chrome
//! directly; chromedriver does that over the Chrome DevTools Protocol.
//!
//! Two ways to get a session, chosen in [`BrowserSession::connect`]:
//!
//! * **Managed** (default): `WebDriver::managed` downloads the chromedriver
//!   that matches the installed Chrome, launches it, and tears it down when
//!   the session ends. Zero setup, but the first run needs internet access.
//! * **External**: if the `WEBDRIVER_URL` environment variable is set, we
//!   connect to a WebDriver server the user started themselves (for example
//!   `chromedriver` running on port 9515). Use this behind a proxy, with
//!   Selenium Grid, or when you want to manage the driver yourself.

use anyhow::{Context, Result};
use thirtyfour::prelude::*;
use tokio::time::{sleep, Duration};

/// How long the browser keeps looking for an element before giving up.
///
/// Set once at connect time; it becomes the implicit wait for every
/// "find element" the session performs. A missing selector therefore fails
/// after this long instead of instantly, which tolerates slow pages.
const IMPLICIT_WAIT: Duration = Duration::from_secs(10);

/// A live browser session: everything the REPL can tell the browser to do.
pub struct BrowserSession {
    driver: WebDriver,
}

impl BrowserSession {
    /// Start Chrome and attach to it.
    ///
    /// Order of precedence:
    /// 1. `WEBDRIVER_URL` set → connect to that externally-managed server.
    /// 2. Otherwise → managed session (auto-downloaded chromedriver).
    pub async fn connect() -> Result<Self> {
        let caps = DesiredCapabilities::chrome();

        let driver = match std::env::var("WEBDRIVER_URL") {
            Ok(url) => {
                // An externally-managed driver: our program just becomes an
                // HTTP client of it. `WebDriver::new` is the shorthand for
                // the default configuration.
                println!("Using WebDriver server at {url}");
                WebDriver::new(&url, caps).await.with_context(|| {
                    format!(
                        "could not reach the WebDriver server at {url} \
                         (is chromedriver running there?)"
                    )
                })?
            }
            Err(_) => {
                // Managed mode: thirtyfour downloads a chromedriver that
                // matches the installed Chrome, starts it, and remembers to
                // stop it when the session ends. First run does a download.
                WebDriver::managed(caps).await.context(
                    "could not start a managed Chrome session \
                     (chromedriver download or launch failed)",
                )?
            }
        };

        // Ask the browser itself to wait for elements, rather than making
        // every REPL command poll manually.
        driver
            .set_implicit_wait_timeout(IMPLICIT_WAIT)
            .await
            .context("could not configure the implicit wait timeout")?;

        Ok(Self { driver })
    }

    /// Navigate the browser to `url`.
    ///
    /// If the user typed a bare host like "example.com" we prepend
    /// `https://` — WebDriver's navigate command needs a scheme. URLs that
    /// already carry one (`http://...`, `data:...`, `about:...`, `file:...`)
    /// are passed through unchanged.
    pub async fn open(&self, url: &str) -> Result<()> {
        let full = if has_scheme(url) {
            url.to_string()
        } else {
            format!("https://{url}")
        };
        self.driver
            .goto(&full)
            .await
            .with_context(|| format!("could not load \"{full}\""))
    }

    /// Click the first element matching the CSS selector.
    pub async fn click(&self, selector: &str) -> Result<()> {
        self.element(selector)
            .await?
            .click()
            .await
            .with_context(|| format!("could not click \"{selector}\""))
    }

    /// Type `text` into the first element matching the selector.
    ///
    /// `send_keys` focuses the element and sends key events for each
    /// character, so the page's own input handlers run just as if a human
    /// typed it. (A faster but less faithful alternative would be setting
    /// the element's `value` property directly via JavaScript.)
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        self.element(selector)
            .await?
            .send_keys(text)
            .await
            .with_context(|| format!("could not type into \"{selector}\""))
    }

    /// Return the visible text of the whole page — the `<body>` element's
    /// text, which is the closest WebDriver equivalent of "what a human
    /// sees when they read the page".
    pub async fn read_page(&self) -> Result<String> {
        self.element("body")
            .await?
            .text()
            .await
            .context("could not read the page text")
    }

    /// Return the visible text of one element. Note that for `<input>`
    /// elements the text is empty — use [`BrowserSession::prop`] to read
    /// their `value` instead.
    pub async fn read_element(&self, selector: &str) -> Result<String> {
        self.element(selector)
            .await?
            .text()
            .await
            .with_context(|| format!("could not read the text of \"{selector}\""))
    }

    /// Read a DOM property (like `value`, `href`, `checked`) of an element.
    /// Returns `None` when the property is not set.
    pub async fn prop(&self, selector: &str, name: &str) -> Result<Option<String>> {
        self.element(selector)
            .await?
            .prop(name)
            .await
            .with_context(|| format!("could not read the \"{name}\" property of \"{selector}\""))
    }

    /// Run `script` in the page and return the boolean it returns.
    ///
    /// This is WebDriver's "Execute Script" endpoint — the escape hatch
    /// for anything the typed commands can't express (inspecting the DOM,
    /// reading computed state). The script must return `true` or `false`;
    /// wrap an arbitrary value in `!!` (as `!!document.querySelector(...)`)
    /// to force a real boolean.
    pub async fn eval_bool(&self, script: &str) -> Result<bool> {
        self.driver
            .execute(script, Vec::new())
            .await
            .with_context(|| "could not run JavaScript in the page")?
            .convert::<bool>()
            .context("the script did not return a boolean (true/false)")
    }

    /// Save a PNG screenshot of the current viewport to `path`.
    pub async fn screenshot(&self, path: &str) -> Result<()> {
        self.driver
            .screenshot(std::path::Path::new(path))
            .await
            .with_context(|| format!("could not save the screenshot to \"{path}\""))
    }

    /// Pause for `secs` seconds. Useful while a slow page finishes loading
    /// before a `click` or `read`.
    pub async fn wait_secs(&self, secs: u64) {
        sleep(Duration::from_secs(secs)).await;
    }

    /// End the WebDriver session and close the browser window.
    ///
    /// Consumes `self` on purpose: after quitting, the session handle is
    /// useless, and the type system should make that impossible to forget.
    pub async fn quit(self) -> Result<()> {
        self.driver
            .quit()
            .await
            .context("could not close the browser cleanly")
    }

    /// Shared helper: find the first element matching a CSS selector.
    ///
    /// thirtyfour's query interface polls until the element appears or the
    /// implicit wait expires, and produces errors that name what we were
    /// looking for — much friendlier than a raw "no such element" trace.
    async fn element(&self, selector: &str) -> Result<WebElement> {
        self.driver
            .query(By::Css(selector))
            .desc(&format!("element matching \"{selector}\""))
            .single()
            .await
            .with_context(|| format!("no element matched \"{selector}\""))
    }
}

/// Does `url` already start with a URL scheme?
///
/// Decision rules (chosen for a REPL where users type short URLs):
/// 1. The part before `:` must look like a scheme — RFC 3986 grammar
///    `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
/// 2. Hierarchical URLs count when `//` follows the colon
///    (`https://example.com`, `file:///C:/x.html`).
/// 3. A few non-hierarchical schemes browsers support in the address bar
///    also count: `data:`, `about:`, `file:`, `mailto:`, `javascript:`,
///    `blob:`.
///
/// Everything else is treated as a bare host and gets `https://`
/// prepended — in particular `localhost:8080`, which matches the scheme
/// grammar but is clearly meant as host-and-port.
///
/// `has_scheme` is deliberately pure (no I/O) so it can be unit-tested.
fn has_scheme(url: &str) -> bool {
    let Some(colon) = url.find(':') else {
        return false;
    };
    let scheme = &url[..colon];
    let valid = {
        let mut chars = scheme.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() => {
                chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            }
            _ => false,
        }
    };
    if !valid {
        return false;
    }
    // `colon` is a byte index of an ASCII ':'; `colon + 1` can equal
    // `url.len()` (for "data:"), and slicing at len is valid (empty).
    if url[colon + 1..].starts_with("//") {
        return true;
    }
    matches!(scheme, "data" | "about" | "file" | "mailto" | "javascript" | "blob")
}

#[cfg(test)]
mod tests {
    use super::has_scheme;

    #[test]
    fn detects_common_schemes() {
        assert!(has_scheme("https://example.com"));
        assert!(has_scheme("http://localhost:9515"));
        assert!(has_scheme("data:text/html,<p>hi</p>"));
        assert!(has_scheme("about:blank"));
        assert!(has_scheme("file:///C:/page.html"));
        assert!(has_scheme("mailto:test@example.com"));
        assert!(has_scheme("data:")); // edge: scheme with nothing after it
    }

    #[test]
    fn rejects_bare_hosts() {
        assert!(!has_scheme("example.com"));
        assert!(!has_scheme("localhost:8080")); // host-and-port, not a scheme
        assert!(!has_scheme("localhost:8080/path"));
        assert!(!has_scheme("www.rust-lang.org/learn"));
    }

    #[test]
    fn rejects_non_scheme_colons() {
        assert!(!has_scheme("8080:9515")); // leading digit
        assert!(!has_scheme(":foo")); // empty scheme
    }
}
