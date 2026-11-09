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
//! Three ways to get a session, chosen in [`BrowserSession::connect`]:
//!
//! * **Attach** (auto-detected): if the user's own Chrome is running with
//!   `--remote-debugging-port` (default port 9222, override via the
//!   `CHROME_DEBUG_ADDRESS` env var), the bot attaches to it via
//!   chromedriver's `debuggerAddress` option and drives the user's
//!   windows — including one they've already logged into Twitter in.
//!   That Chrome is never closed by the bot.
//! * **Managed** (default): `WebDriver::managed` downloads the chromedriver
//!   that matches the installed Chrome, launches a fresh Chrome, and tears
//!   it down when the session ends. Zero setup, but the first run needs
//!   internet access.
//! * **External**: if the `WEBDRIVER_URL` environment variable is set, we
//!   connect to a WebDriver server the user started themselves (for example
//!   `chromedriver` running on port 9515). Use this behind a proxy, with
//!   Selenium Grid, or when you want to manage the driver yourself.

use anyhow::{Context, Result};
use thirtyfour::{prelude::*, ChromeCapabilities, ChromiumLikeCapabilities};
use tokio::time::{sleep, Duration};

/// How long the browser keeps looking for an element before giving up.
///
/// Set once at connect time; it becomes the implicit wait for every
/// "find element" the session performs. A missing selector therefore fails
/// after this long instead of instantly, which tolerates slow pages.
const IMPLICIT_WAIT: Duration = Duration::from_secs(10);

/// Default address Chrome's DevTools listens on with
/// `--remote-debugging-port` when no explicit port is given.
const DEFAULT_DEBUG_ADDRESS: &str = "127.0.0.1:9222";

/// A live browser session: everything the REPL can tell the browser to do.
pub struct BrowserSession {
    driver: WebDriver,
    /// True when the session attached to the user's own Chrome instead of
    /// spawning a fresh one. Drives quit() semantics: attached browsers
    /// belong to the user and must not be closed.
    attached: bool,
}

impl BrowserSession {
    /// Start Chrome and attach to it.
    ///
    /// Order of precedence:
    /// 1. `WEBDRIVER_URL` set → connect to that externally-managed server.
    /// 2. A remote-debugging port is open (default `127.0.0.1:9222`,
    ///    override with `CHROME_DEBUG_ADDRESS`) → attach to the user's
    ///    Chrome there.
    /// 3. Otherwise → fresh managed Chrome (auto-downloaded chromedriver).
    ///
    /// Step 2 is best-effort: a refused probe or a failed attach falls
    /// through to step 3 with a notice, never an error.
    pub async fn connect() -> Result<Self> {
        // 1. External WebDriver server.
        if let Ok(url) = std::env::var("WEBDRIVER_URL") {
            println!("Using WebDriver server at {url}");
            let driver = WebDriver::new(&url, DesiredCapabilities::chrome())
                .await
                .with_context(|| {
                    format!(
                        "could not reach the WebDriver server at {url} \
                         (is chromedriver running there?)"
                    )
                })?;
            driver
                .set_implicit_wait_timeout(IMPLICIT_WAIT)
                .await
                .context("could not configure the implicit wait timeout")?;
            return Ok(Self { driver, attached: false });
        }

        // 2. Attach to the user's Chrome, if present. chromedriver's
        //    `debuggerAddress` option makes it attach to an already
        //    running browser instead of spawning one; the browser is
        //    user-launched, so it carries their logins.
        let debug_address = std::env::var("CHROME_DEBUG_ADDRESS")
            .unwrap_or_else(|_| DEFAULT_DEBUG_ADDRESS.to_string());
        if debug_port_open(&debug_address) {
            let mut caps = ChromeCapabilities::new();
            if caps.set_debugger_address(&debug_address).is_ok() {
                match WebDriver::managed(caps).await {
                    Ok(driver) => {
                        println!(
                            "Attached to your Chrome at {debug_address} \
                             (it stays open when the bot exits)."
                        );
                        driver
                            .set_implicit_wait_timeout(IMPLICIT_WAIT)
                            .await
                            .context("could not configure the implicit wait timeout")?;
                        return Ok(Self { driver, attached: true });
                    }
                    Err(err) => {
                        println!(
                            "note: something is listening at {debug_address} but \
                             attaching failed ({err}); using a fresh Chrome instead."
                        );
                    }
                }
            }
        }

        // 3. Fresh managed Chrome — the zero-setup default. First run
        //    downloads the matching chromedriver.
        let driver = WebDriver::managed(DesiredCapabilities::chrome()).await.context(
            "could not start a managed Chrome session \
             (chromedriver download or launch failed)",
        )?;
        driver
            .set_implicit_wait_timeout(IMPLICIT_WAIT)
            .await
            .context("could not configure the implicit wait timeout")?;
        Ok(Self { driver, attached: false })
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

    /// Handles of all browser windows in the session.
    ///
    /// Chromium reports each tab as its own "window" handle. Attach mode
    /// uses this list to find the user's Twitter window.
    pub async fn window_handles(&self) -> Result<Vec<WindowHandle>> {
        self.driver
            .windows()
            .await
            .context("could not list the browser windows")
    }

    /// The URL of the window the browser is currently driving.
    pub async fn current_url(&self) -> Result<String> {
        self.driver
            .current_url()
            .await
            .context("could not read the current URL")
            .map(|url| url.to_string())
    }

    /// Make `handle` the window the browser drives next.
    pub async fn switch_to_window(&self, handle: &WindowHandle) -> Result<()> {
        self.driver
            .switch_to_window(handle.clone())
            .await
            .context("could not switch windows")
    }

    /// Enumerate windows as (handle, current URL), then restore the
    /// window that was active before the call.
    ///
    /// Reading a window's URL requires switching to it briefly, so the
    /// restore at the end keeps the user's visible window untouched.
    pub async fn windows_with_urls(&self) -> Result<Vec<(WindowHandle, String)>> {
        let original = self
            .driver
            .window()
            .await
            .context("could not read the current window")?;

        let mut out = Vec::new();
        for handle in self
            .driver
            .windows()
            .await
            .context("could not list the browser windows")?
        {
            self.driver
                .switch_to_window(handle.clone())
                .await
                .context("could not switch windows while enumerating")?;
            let url = self
                .driver
                .current_url()
                .await
                .context("could not read a window's URL")?
                .to_string();
            out.push((handle, url));
        }

        self.driver
            .switch_to_window(original)
            .await
            .context("could not switch back to the original window")?;
        Ok(out)
    }

    /// Open a new tab (Chromium lists it as a new window handle).
    pub async fn new_tab(&self) -> Result<()> {
        self.driver
            .new_tab()
            .await
            .context("could not open a new tab")?;
        Ok(())
    }

    /// End the WebDriver session.
    ///
    /// Consumes `self` on purpose: after quitting, the session handle is
    /// useless, and the type system should make that impossible to forget.
    ///
    /// A fresh (bot-spawned) Chrome is closed. An *attached* Chrome
    /// belongs to the user and survives: chromedriver ends the session
    /// without killing a browser it attached to.
    pub async fn quit(self) -> Result<()> {
        let was_attached = self.attached;
        self.driver
            .quit()
            .await
            .context("could not end the browser session cleanly")?;
        if was_attached {
            println!("Detached from your Chrome (it stays open).");
        }
        Ok(())
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

/// Is something listening on the Chrome remote-debugging address?
///
/// A plain TCP probe — std only, no new dependencies, ~100 ms worst
/// case. Used to decide whether to try attaching to the user's Chrome.
/// A refused connection (no debug Chrome running) is the common case and
/// must stay fast and silent.
fn debug_port_open(addr: &str) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return false; // malformed address → treat as "not running"
    };
    let Some(first) = addrs.next() else {
        return false;
    };
    std::net::TcpStream::connect_timeout(&first, Duration::from_millis(100)).is_ok()
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
