//! The `browser_bot` binary: a thin shell around the library.
//!
//! Responsibilities of `main` (and only `main`):
//! 1. Start the Tokio async runtime (via the `#[tokio::main]` attribute).
//! 2. Connect to the browser — fail fast, before showing a prompt.
//! 3. Hand control to the REPL loop.
//! 4. Close the browser when the loop ends — on every path.

use anyhow::Result;
use browser_bot::{browser::BrowserSession, repl};

/// `#[tokio::main]` rewrites this function so the body runs inside a Tokio
/// multi-threaded runtime. That matters because every WebDriver call is
/// asynchronous I/O: awaiting a response from chromedriver should never
/// block the thread that is waiting for the next line of terminal input.
#[tokio::main]
async fn main() -> Result<()> {
    println!("browser_bot v{}", env!("CARGO_PKG_VERSION"));
    println!("Connecting to Chrome (first run may download chromedriver)...");

    // Fail fast: there's no point showing a prompt if the browser can't be
    // reached. `connect` returns an error with an actionable message.
    let mut session = BrowserSession::connect().await?;

    println!("Ready. Type `help` for the command list, `quit` to exit.\n");

    // The REPL loop blocks (in the async sense) until the user quits via
    // `quit`, Ctrl+C, or end-of-input (Ctrl+D on Unix, Ctrl+Z on Windows).
    // The loop handles user errors itself; only a real terminal I/O
    // failure comes back as `Err`. Either way, the browser must be closed
    // first, so both results are held and reported after shutdown.
    let run_result = repl::run(&mut session).await;
    let quit_result = session.quit().await;
    println!("Browser closed. Goodbye.");
    run_result?;
    quit_result?;
    Ok(())
}
