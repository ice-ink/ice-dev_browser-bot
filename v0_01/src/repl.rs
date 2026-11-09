//! The interactive terminal loop and the command parser.
//!
//! # Shape of the REPL
//!
//! ```text
//! browser-bot> open https://example.com
//! browser-bot> read
//! Example Domain
//! browser-bot> quit
//! ```
//!
//! Each line goes through two stages:
//! 1. [`parse`] turns the line into a [`Command`] (pure, no I/O, unit-tested).
//! 2. [`run`] executes the command against the [`BrowserSession`] and
//!    prints the result — or a friendly error, never a crash.
//!
//! The loop awaits two things at once with `tokio::select!`: the next line
//! of input and a Ctrl+C signal. Whichever arrives first wins the race.
//! This is the same idea as `select(2)`/`poll(2)` on Unix — one thread
//! waiting efficiently on several event sources.

use std::io::Write;

use anyhow::Result;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::signal;

use crate::{browser::BrowserSession, subroutines};

/// One parsed instruction, ready to be executed against the browser.
#[derive(Debug, PartialEq)]
pub enum Command {
    /// `open <url>` — navigate the browser.
    Open(String),
    /// `click <css-selector>` — click the first matching element.
    Click(String),
    /// `type <css-selector> <text>` — type into the first matching element.
    Type { selector: String, text: String },
    /// `read [css-selector]` — print page text, or one element's text.
    Read(Option<String>),
    /// `screenshot <path>` — save a PNG of the current viewport.
    Screenshot(String),
    /// `wait <seconds>` — pause, e.g. for a slow page load.
    Wait(u64),
    /// `windows` — list the browser's windows (handle → URL). Useful in
    /// attach mode to see what the bot can drive.
    Windows,
    /// `run <name>` — execute a named subroutine (a multi-step flow; see
    /// `help` for the list). Blocks the prompt until it finishes.
    Run(String),
    /// `help` — print the command list.
    Help,
    /// `quit` — close the browser and exit.
    Quit,
}

/// Split a line into words, honoring double quotes.
///
/// `type "#q" "rust lang"` → `["type", "#q", "rust lang"]`
///
/// Quoting rules (deliberately tiny):
/// * `"` opens a quoted token and the next `"` closes it.
/// * Whitespace outside quotes separates tokens.
/// * A quote in the middle of a token is kept literally.
/// * An unterminated quote runs to the end of the line.
pub fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in line.chars() {
        match c {
            '"' if !in_quotes && current.is_empty() => in_quotes = true,
            '"' if in_quotes => in_quotes = false,
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    // `take` replaces `current` with an empty string and
                    // hands us the old value, avoiding a clone.
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parse one line of user input.
///
/// Returns `Err` with a short usage message for malformed lines — the REPL
/// prints it and keeps going. Parsing is a pure function of the string so
/// it can be unit-tested without a browser.
pub fn parse(line: &str) -> Result<Command, String> {
    let tokens = tokenize(line);
    let (cmd, args) = tokens
        .split_first()
        .ok_or_else(|| "type a command — `help` lists them".to_string())?;

    match cmd.as_str() {
        "open" => {
            if args.is_empty() {
                return Err("usage: open <url>".to_string());
            }
            // Join the rest of the line back into one URL. Quoted tokens
            // keep embedded spaces; joining also reassembles data: URLs
            // whose markup contains spaces (`<input id="f">` etc.).
            // A URL that *needs* a literal space must use %20 or quoting.
            Ok(Command::Open(args.join(" ")))
        }
        "click" => {
            let sel = args.first().ok_or("usage: click <css-selector>")?;
            Ok(Command::Click(sel.clone()))
        }
        "type" => {
            let sel = args.first().ok_or("usage: type <css-selector> <text>")?;
            let text = args[1..].join(" ");
            if text.is_empty() {
                return Err("usage: type <css-selector> <text>".to_string());
            }
            Ok(Command::Type { selector: sel.clone(), text })
        }
        "read" => Ok(Command::Read(args.first().cloned())),
        "screenshot" => {
            let path = args.first().ok_or("usage: screenshot <path.png>")?;
            Ok(Command::Screenshot(path.clone()))
        }
        "wait" => {
            let raw = args.first().ok_or("usage: wait <seconds>")?;
            let secs: u64 = raw
                .parse()
                .map_err(|_| format!("wait needs a whole number of seconds, got \"{raw}\""))?;
            Ok(Command::Wait(secs))
        }
        "run" => {
            let name = args.first().ok_or("usage: run <subroutine>")?;
            if args.len() > 1 {
                return Err("usage: run <subroutine> — one name only".to_string());
            }
            Ok(Command::Run(name.clone()))
        }
        "windows" => Ok(Command::Windows),
        "help" => Ok(Command::Help),
        "quit" | "exit" => Ok(Command::Quit),
        other => Err(format!("unknown command \"{other}\" — `help` lists them")),
    }
}

/// Run the interactive loop until the user quits.
///
/// Returns normally on `quit`, Ctrl+C, or end-of-input (Ctrl+D on Unix,
/// Ctrl+Z on Windows). The caller is responsible for closing the browser.
pub async fn run(session: &mut BrowserSession) -> Result<()> {
    // Tokio's async stdin lets us await a line without blocking the whole
    // runtime; the blocking std::io::stdin().read_line() would freeze it.
    let mut reader = BufReader::new(tokio::io::stdin());
    let mut line = String::new();

    loop {
        // Prompt first, flush immediately: stdout is buffered, and without
        // the flush the user would stare at an empty terminal.
        print!("browser-bot> ");
        std::io::stdout().flush().ok();

        line.clear();
        tokio::select! {
            // `read_line` appends to `line` and returns the byte count;
            // Ok(0) means end-of-input (Ctrl+D / Ctrl+Z).
            read = reader.read_line(&mut line) => {
                let n = read?;
                if n == 0 {
                    println!(); // leave the prompt on its own line
                    break;
                }
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue; // just Enter — reprompt
                }
                if !dispatch(session, trimmed).await {
                    break;
                }
            }
            // Ctrl+C: the signal arrives as a future we can race against
            // the input future. This only works with the "signal" feature.
            _ = signal::ctrl_c() => {
                println!();
                println!("Interrupted — closing the browser.");
                break;
            }
        }
    }
    Ok(())
}

/// Execute one parsed line. Returns `false` when the user asked to quit.
async fn dispatch(session: &mut BrowserSession, line: &str) -> bool {
    match parse(line) {
        Ok(Command::Help) => print_help(),
        Ok(Command::Quit) => return false,
        Ok(Command::Open(url)) => exec(session.open(&url).await),
        Ok(Command::Click(selector)) => exec(session.click(&selector).await),
        Ok(Command::Type { selector, text }) => {
            exec(session.type_text(&selector, &text).await)
        }
        Ok(Command::Read(None)) => match session.read_page().await {
            Ok(text) => println!("{text}"),
            Err(err) => eprintln!("error: {err:#}"),
        },
        Ok(Command::Read(Some(selector))) => match session.read_element(&selector).await {
            Ok(text) => println!("{text}"),
            Err(err) => eprintln!("error: {err:#}"),
        },
        Ok(Command::Screenshot(path)) => match session.screenshot(&path).await {
            Ok(()) => println!("saved screenshot to \"{path}\""),
            Err(err) => eprintln!("error: {err:#}"),
        },
        Ok(Command::Wait(secs)) => {
            println!("waiting {secs}s...");
            session.wait_secs(secs).await;
        }
        Ok(Command::Windows) => match session.windows_with_urls().await {
            Ok(windows) => {
                println!("windows:");
                for (handle, url) in windows {
                    println!("  {handle} -> {url}");
                }
            }
            Err(err) => eprintln!("error: {err:#}"),
        },
        Ok(Command::Run(name)) => {
            // Subroutines can block for minutes (waiting for a human to
            // log in). Race the subroutine against Ctrl+C so the user can
            // cancel *just* this command without quitting the program.
            // Dropping the subroutine future mid-WebDriver-call is safe:
            // the next command simply starts fresh.
            tokio::select! {
                result = subroutines::execute(session, &name) => exec(result),
                _ = signal::ctrl_c() => {
                    println!();
                    println!("subroutine cancelled — back to the prompt.");
                }
            }
        }
        Err(message) => eprintln!("error: {message}"),
    }
    true
}

/// Run a fallible browser operation and print its error chain instead of
/// crashing. `{err:#}` is anyhow's "alternate" format, which prints every
/// cause in the chain — one line per `with_context`.
///
/// This takes the *result* (callers await the operation first) because it
/// has nothing to await itself; an `async fn` that never awaits would be
/// a needless indirection.
fn exec(result: anyhow::Result<()>) {
    if let Err(err) = result {
        eprintln!("error: {err:#}");
    }
}

/// The command reference shown by `help`.
///
/// The subroutines section is printed from [`subroutines::registry`]
/// rather than hard-coded, so adding a subroutine updates the help text
/// automatically.
fn print_help() {
    println!(
        "\
commands:
  open <url>                    navigate the browser (https:// assumed if omitted)
  click <css-selector>          click the first matching element
  type <css-selector> <text>    type into the first matching element (\"quote text\")
  read [css-selector]           print the page text, or one element's text
  screenshot <path.png>         save a PNG of the current viewport
  wait <seconds>                pause (for slow page loads)
  windows                       list the browser's windows (handle → URL)
  help                          show this list
  quit                          close the browser and exit"
    );
    println!();
    println!("subroutines:");
    for (name, description) in subroutines::registry() {
        println!("  run {name:<26} {description}");
    }
    println!();
    println!("`run` blocks the prompt until the subroutine finishes (Ctrl+C cancels it).");
    println!();
    println!("exit anytime with quit, Ctrl+C, or Ctrl+D (Ctrl+Z on Windows)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_on_whitespace() {
        assert_eq!(
            tokenize("open https://example.com"),
            vec!["open", "https://example.com"]
        );
    }

    #[test]
    fn tokenize_keeps_quoted_text_whole() {
        // r##"..."## (raw string with a two-hash delimiter): the content
        // contains `"#q"` whose `"#` would terminate a plain r#"..."#.
        assert_eq!(
            tokenize(r##"type "#q" "rust lang""##),
            vec!["type", "#q", "rust lang"]
        );
    }

    #[test]
    fn tokenize_unterminated_quote_runs_to_end() {
        assert_eq!(
            tokenize("type #q \"rust lang"),
            vec!["type", "#q", "rust lang"]
        );
    }

    #[test]
    fn tokenize_splits_on_markup_spaces() {
        // Spaces between HTML attributes split tokens; `open` joins them
        // back (see parse_open_joins_the_rest_of_the_line).
        let line = r#"open data:text/html,<input id="f"><p id="o">hi</p>"#;
        assert_eq!(
            tokenize(line),
            vec!["open", "data:text/html,<input", "id=\"f\"><p", "id=\"o\">hi</p>"]
        );
    }

    #[test]
    fn parse_open_joins_the_rest_of_the_line() {
        let line = r#"open data:text/html,<input id="f"><p id="o">hi</p>"#;
        assert_eq!(
            parse(line),
            Ok(Command::Open(r#"data:text/html,<input id="f"><p id="o">hi</p>"#.into()))
        );
    }

    #[test]
    fn parse_open() {
        assert_eq!(
            parse("open https://example.com"),
            Ok(Command::Open("https://example.com".into()))
        );
    }

    #[test]
    fn parse_type_joins_unquoted_tail() {
        assert_eq!(
            parse("type #q hello world"),
            Ok(Command::Type { selector: "#q".into(), text: "hello world".into() })
        );
    }

    #[test]
    fn parse_type_with_quotes() {
        assert_eq!(
            parse(r##"type "#q" "hello world""##),
            Ok(Command::Type { selector: "#q".into(), text: "hello world".into() })
        );
    }

    #[test]
    fn parse_read_without_selector() {
        assert_eq!(parse("read"), Ok(Command::Read(None)));
    }

    #[test]
    fn parse_read_with_selector() {
        assert_eq!(parse("read #title"), Ok(Command::Read(Some("#title".into()))));
    }

    #[test]
    fn parse_screenshot() {
        assert_eq!(
            parse("screenshot shot.png"),
            Ok(Command::Screenshot("shot.png".into()))
        );
    }

    #[test]
    fn parse_wait() {
        assert_eq!(parse("wait 3"), Ok(Command::Wait(3)));
    }

    #[test]
    fn parse_help_and_quit() {
        assert_eq!(parse("help"), Ok(Command::Help));
        assert_eq!(parse("quit"), Ok(Command::Quit));
        assert_eq!(parse("exit"), Ok(Command::Quit));
    }

    #[test]
    fn parse_run() {
        assert_eq!(
            parse("run twitter-login"),
            Ok(Command::Run("twitter-login".into()))
        );
    }

    #[test]
    fn parse_run_rejects_missing_or_extra_names() {
        assert!(parse("run").unwrap_err().contains("usage"));
        assert!(parse("run twitter-login extra").unwrap_err().contains("one name"));
    }

    #[test]
    fn parse_windows() {
        assert_eq!(parse("windows"), Ok(Command::Windows));
    }

    #[test]
    fn parse_errors_are_friendly() {
        assert!(parse("").is_err());
        assert!(parse("frobnicate").unwrap_err().contains("unknown command"));
        assert!(parse("open").unwrap_err().contains("usage"));
        assert!(parse("click").unwrap_err().contains("usage"));
        assert!(parse("type #q").unwrap_err().contains("usage"));
        assert!(parse("screenshot").unwrap_err().contains("usage"));
        assert!(parse("wait soon").unwrap_err().contains("whole number"));
        assert!(parse("wait").unwrap_err().contains("usage"));
    }
}
