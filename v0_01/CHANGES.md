# Changes

Per the agent instructions: since this is alpha, every version documents
what changed — for someone learning the language and the software.

## v0_01 — first alpha build

Created the first version of browser_bot: a terminal-controlled browser
agent in Rust. The user types instructions at a prompt; the program
carries them out in a real Chrome window.

What's new:

- Interactive terminal REPL — the primary user interface, per the agent
  instructions.
- Drives real Chrome through the W3C WebDriver protocol, using the
  thirtyfour crate (async, Tokio).
- Managed chromedriver: auto-downloads and runs the driver matching the
  installed Chrome; an external driver can be used instead via the
  `WEBDRIVER_URL` environment variable.
- Commands: `open`, `click`, `type`, `read`, `screenshot`, `wait`,
  `help`, `quit`. Quoted text supported; `open` reassembles the rest of
  the line into the URL, and bare hosts get `https://` prepended.
- Friendly errors: bad input or missing elements print a message with
  the cause chain; the REPL never crashes. Chrome is closed cleanly on
  every exit path (`quit`, Ctrl+C, end-of-input).
- 10-second implicit wait so slow pages don't cause spurious failures.
- Tests: unit tests for the command parser, plus a live-browser
  integration test that skips gracefully when no browser is available.
- Documentation: README user guide, architecture walkthrough
  (`docs/architecture.md`), this changelog, and the v0_02 roadmap.

Known limitations in v0_01 (see roadmap):

- No script/batch mode, config file, or headless flag.
- `read` on an `<input>` shows nothing (inputs have no element text).
- Only the first matching element per selector is used.
- No command history or line editing.
- Fixed-time `wait` rather than condition-based waits.

## v0_01 update — subroutines category (`run`)

Added a subroutines category of commands: multi-step flows invoked with
`run <name>`, blocking the prompt until they finish. The first
subroutine is `run twitter-login`.

What's new:

- `src/subroutines.rs` — registry of named subroutines; `help` prints
  it, `check_known` validates names, `execute` dispatches.
- `run twitter-login` — opens Twitter (twitter.com → x.com) in the
  browser and blocks the REPL while polling the page every 2 seconds
  until the login wall disappears, i.e. until the user has logged in by
  hand. 10-minute timeout with a friendly error; Ctrl+C cancels just
  the subroutine. Login markers verified against the live x.com markup
  (Nov 2026); the agent never sees credentials.
- `BrowserSession::eval_bool` — Execute Script bridge for DOM probes
  (the groundwork for a future `js` command).
- Ctrl+C during `run` cancels the subroutine instead of exiting the
  program (`tokio::select!` in the dispatcher).
- Tests: unit tests for `run` parsing and the registry; live-browser
  integration tests that fake the login transition with a `data:` URL
  and a JS timer (detection, already-logged-in, and timeout paths) — no
  account or network needed.
- Docs: README subroutines section; architecture walkthrough of the
  registry, the poll-until-condition loop, and cancellation.

Known limitations of this update:

- Login detection relies on x.com's current logged-out markup; if x.com
  redesigns, the markers in `src/subroutines.rs` need updating (Ctrl+C
  and the timeout remain the escapes).
- Lines typed during a running subroutine are buffered and execute
  afterwards, not discarded.

## Roadmap for the next version

See [docs/roadmap-v0_02.md](docs/roadmap-v0_02.md).
