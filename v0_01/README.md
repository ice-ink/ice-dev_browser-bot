# browser_bot v0_01

A terminal-controlled browser agent, written in Rust. You type short
instructions at a prompt and the program carries them out in a real Chrome
window — open pages, click, type, read text, take screenshots.

browser_bot talks to Chrome through the WebDriver protocol (via the
[thirtyfour] crate): your commands become HTTP requests to chromedriver,
which relays them to Chrome. This is the first alpha version of a larger
goal — an agent that acts out the instructions it is given, controlled
from a terminal.

[thirtyfour]: https://docs.rs/thirtyfour

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (built with 1.99)
- [Google Chrome](https://www.google.com/chrome/)
- Internet access on the first run (chromedriver is downloaded
  automatically — see below)

No chromedriver installation is needed: on first start, thirtyfour's
managed mode downloads the chromedriver that matches your installed Chrome
version and starts it for you. If you prefer to manage chromedriver
yourself, see [Using your own chromedriver](#using-your-own-chromedriver).

## Quick start

From this directory (`v0_01`):

```
cargo run
```

Chrome opens and you get a prompt:

```
browser_bot v0.1.0
Connecting to Chrome (first run may download chromedriver)...
Ready. Type `help` for the command list, `quit` to exit.

browser-bot> open https://example.com
browser-bot> read
Example Domain
...
browser-bot> screenshot example.png
saved screenshot to "example.png"
browser-bot> quit
Browser closed. Goodbye.
```

## Command reference

| Command | What it does |
| --- | --- |
| `open <url>` | Navigate the browser. `https://` is assumed if you omit the scheme (`open example.com` works). |
| `click <css-selector>` | Click the first element matching the CSS selector. |
| `type <css-selector> <text>` | Type text into the first matching element, as if typed by a person. Quote text with spaces: `type "#q" "rust lang"`. |
| `read [css-selector]` | Print the visible text of the whole page, or of one element. |
| `screenshot <path.png>` | Save a PNG screenshot of the current viewport. |
| `wait <seconds>` | Pause — handy while a slow page finishes loading. |
| `help` | Show the command list. |
| `quit` | Close the browser and exit. |

Exit anytime with `quit`, Ctrl+C, or Ctrl+D (Ctrl+Z on Windows).

### Subroutines

Subroutines are multi-step flows, run with `run <name>`. While one runs,
the prompt is blocked — no other instruction is processed until it
finishes.

| Command | What it does |
| --- | --- |
| `run twitter-login` | Opens Twitter in the browser and waits — polling the page every 2 seconds — until you've logged in. |

`run twitter-login` in detail: navigates to https://twitter.com (which
redirects to x.com), then blocks the prompt and watches the page until
the login wall is gone. It gives up after 10 minutes with an error.
Type Ctrl+C to cancel it and get the prompt back; lines you type while
it runs are held and executed afterwards. It never sees your
credentials — you log in by hand in the browser window.

Login detection looks for the logged-out login wall's page elements
(verified against x.com's markup in Nov 2026). x.com is a third-party
site that changes its markup; if detection misbehaves, Ctrl+C and the
timeout are the escapes, and the markers live in
`src/subroutines.rs` where they're easy to update.

### Selectors

`click`, `type`, and `read` take CSS selectors: `#id`, `.class`,
`button[type="submit"]`, `ul > li`, and so on.
[MDN's CSS selectors guide](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_selectors)
is the primary reference.

### Notes and limits (v0_01)

- If an element isn't found, the command waits up to 10 seconds (the
  implicit wait) and then prints `error: no element matched "..."` — the
  REPL never crashes on a bad selector.
- `read` on an `<input>` prints nothing (inputs have no text — a WebDriver
  rule). A `value` command is planned for v0_02.
- Only the first matching element per selector is used. To target a
  specific element among many, make the selector more precise.
- URLs can't contain raw spaces. Encode them as `%20`, or quote the whole
  URL: `open "https://example.com/a page"`. Spaces in *markup* are fine —
  `open` reassembles the line, so `data:` URLs (handy for practicing
  without a server) work as typed:
  `open data:text/html,<input id="f"><p id="o">hi</p>`.
- Unknown subroutine names print the available list: `run <name>`
  validates against the registry in `src/subroutines.rs`.
- The program always closes Chrome cleanly on exit, including Ctrl+C.

## Using your own chromedriver

If you prefer to run chromedriver yourself (behind a proxy, or pinned to a
specific version), set the `WEBDRIVER_URL` environment variable before
starting browser_bot. The program then connects to that server instead of
managing its own:

```
chromedriver --port=9515            # in another terminal
$env:WEBDRIVER_URL = "http://localhost:9515"    # PowerShell
cargo run
```

[ChromeDriver](https://developer.chrome.com/docs/chromedriver) speaks the
WebDriver protocol on port 9515 by default.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `could not start a managed Chrome session` | Install Chrome, or check your internet connection on the first run (chromedriver download). |
| `could not reach the WebDriver server at ...` | With `WEBDRIVER_URL` set: make sure chromedriver is running at that address. |
| `no element matched "..."` | The page didn't show that selector within 10s. Check the selector, or `wait` before the command. |
| Browser opens but nothing seems to happen | The page may still be loading — `wait 2` then retry. |

## Where to learn more

- [docs/architecture.md](docs/architecture.md) — how the code works,
  written for programmers learning Rust and WebDriver.
- [CHANGES.md](CHANGES.md) — what changed in v0_01.
- [docs/roadmap-v0_02.md](docs/roadmap-v0_02.md) — where v0_02 is heading.

## Primary sources

- [W3C WebDriver specification](https://w3c.github.io/webdriver/) — the
  protocol browser_bot speaks.
- [thirtyfour](https://docs.rs/thirtyfour) and the
  [thirtyfour book](https://stevepryde.github.io/thirtyfour/) — the Rust
  crate that implements the protocol client.
- [ChromeDriver](https://developer.chrome.com/docs/chromedriver) — the
  driver that translates WebDriver into Chrome commands.
- [MDN CSS selectors](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_selectors)
  — the selector syntax for `click`/`type`/`read`.
