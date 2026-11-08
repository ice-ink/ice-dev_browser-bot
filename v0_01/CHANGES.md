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

## Roadmap for the next version

See [docs/roadmap-v0_02.md](docs/roadmap-v0_02.md).
