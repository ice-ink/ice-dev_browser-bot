# browser_bot v0_01 — architecture and code walkthrough

Written for someone who knows programming fundamentals and wants to
understand this codebase — and pick up Rust, async I/O, and WebDriver
along the way. Read it with the source open; every section points at the
files it describes.

## The big picture

```
┌─────────────┐ typed line  ┌──────────────┐ run <name>  ┌──────────────────┐ method call ┌───────────────┐ HTTP ┌──────────────┐ CDP ┌────────┐
│  terminal   │ ──────────▶ │  src/repl.rs │ ──────────▶ │ src/subroutines.rs│ ──────────▶ │ src/browser.rs │ ────▶ │ chromedriver │ ────▶ │ Chrome │
└─────────────┘             └──────────────┘             └──────────────────┘             └───────────────┘      └──────────────┘      └────────┘
```

- **Terminal** — the primary user interface, by design. The user types
  instructions like `open https://example.com` or `click #submit`.
- **`src/repl.rs`** — reads lines, turns them into `Command` values
  (parsing), and executes the single-step commands; `run <name>` is
  dispatched onward. Knows nothing about browsers.
- **`src/subroutines.rs`** — multi-step flows: navigate, poll the page,
  wait for a human. Knows nothing about terminals; speaks to the browser
  only through `BrowserSession`.
- **`src/browser.rs`** — `BrowserSession`, a thin wrapper around the
  [thirtyfour](https://docs.rs/thirtyfour) crate's `WebDriver`. Knows
  nothing about terminals or task flows.
- **chromedriver** — a small server program. Receives WebDriver HTTP
  requests and translates them into Chrome DevTools Protocol (CDP)
  commands that Chrome understands. Chrome is the only one who can drive
  Chrome; chromedriver is the translator.
- **Chrome** — the browser window that actually does the work.

The seam between `repl` and `browser` is deliberate: the REPL is just one
possible frontend. A script mode (planned for v0_02) would reuse
`BrowserSession` unchanged.

## Crate layout

`src/lib.rs` declares the library
(`pub mod browser; pub mod repl; pub mod subroutines;`) and `src/main.rs`
is the binary. Why split a program this small into a library and a
binary? Two reasons, both on display in `src/lib.rs`'s doc comment:

1. **Testability.** Rust integration tests (in `tests/`) can `use`
   library crates but not binaries. The live-browser test in
   `tests/browser_integration.rs` imports `browser_bot::browser` directly.
2. **Reuse.** The planned script mode gets the same library for free.

`src/main.rs` is intentionally boring: start the runtime, connect, run the
REPL, quit. Keeping `main` thin is a Rust convention worth internalizing —
anything you can't test in `main` belongs in the library.

## Why async? (tokio, `#[tokio::main]`, `await`)

Every WebDriver call is I/O: an HTTP request that takes milliseconds to
seconds. If the program waited for each one the old synchronous way, it
would still work — but the terminal would freeze during every command,
and nothing else could happen while the browser thinks.

Rust models this with **futures**. A future is "a value that will be ready
later." When you write `driver.goto(url).await`, you are saying: pause
this function until the future completes, and let other tasks use the
thread meanwhile. That pausing is cooperative — it only works if
something schedules and drives the futures. That something is an async
runtime; this project uses [Tokio](https://tokio.rs/).

`#[tokio::main]` in `src/main.rs` is the entry point: it creates a
multi-threaded Tokio runtime and runs `main` inside it. The feature flags
in `Cargo.toml` (`rt-multi-thread`, `macros`, `io-util`, `signal`, `time`)
enable exactly the pieces used:

- `rt-multi-thread` — the runtime itself.
- `macros` — `#[tokio::main]` and `#[tokio::test]`.
- `io-util` — Tokio's async stdin (`repl.rs`).
- `signal` — Ctrl+C handling (`repl.rs`).
- `time` — the `sleep` behind the `wait` command (`browser.rs`).

To go deeper: The Rust Book's
[async chapter](https://doc.rust-lang.org/book/ch20-00-final-project-a-web-server.html)
and the [Tokio tutorial](https://tokio.rs/tokio/tutorial) are the primary
sources.

## `src/repl.rs` — parsing and the loop

### Parsing

`parse` is a **pure function**: a string in, a `Command` out (or a
`Result::Err` with a usage message). Pure functions are the easiest thing
in the world to test — see the `#[cfg(test)] mod tests` block, which needs
no browser and no runtime. That's the whole reason parsing is separated
from executing.

`tokenize` is a tiny hand-rolled tokenizer with one job: keep quoted
strings whole. `type "#q" "rust lang"` must become three tokens, not
four. The rules are documented in its doc comment; the interesting Rust
detail is `std::mem::take(&mut current)`, which replaces `current` with
an empty `String` and returns the old value — moving it into the token
list without a clone. (`String` owns its heap buffer, so clones cost an
allocation; `take` moves the buffer instead.)

### The loop

`run` shows the prompt and then awaits two futures at once via
`tokio::select!`:

```rust
tokio::select! {
    read = reader.read_line(&mut line) => { ... }
    _ = signal::ctrl_c() => { ... }
}
```

`select!` polls both futures; the first one to complete wins, and the
other is dropped. This is the same pattern as `select(2)`/`poll(2)` in
Unix systems programming — one thread waiting efficiently on several
event sources. Notice what this buys us: Ctrl+C works even while the
program is waiting for input, because waiting is an awaitable future, not
a blocking call.

`read_line` returning `Ok(0)` means **end of input** — the user pressed
Ctrl+D (Unix) or Ctrl+Z (Windows). The loop treats that as a quit.

### Errors

Two kinds of bad input exist, and they're handled differently:

- **Bad line** (`parse` returns `Err`): print the message, keep looping.
- **Bad browser outcome** (element missing, page failed to load): the
  `?` operator inside `BrowserSession` turns it into an `anyhow::Error`
  with context, and `exec`/`dispatch` print the whole chain with
  `{err:#}`. The chain is what makes messages like
  `error: could not click "#submit": no element matched "#submit"`
  possible — each `with_context` in `browser.rs` adds one link.

Nothing panics, because a bad instruction is an everyday event, not a bug.

## `src/browser.rs` — the WebDriver layer

### The protocol in one breath

The [W3C WebDriver spec](https://w3c.github.io/webdriver/) defines a
REST-ish protocol for automating browsers. Sessions, navigation, and
element operations are all HTTP endpoints, for example:

| You want | WebDriver command | HTTP |
| --- | --- | --- |
| A session | New Session | `POST /session` |
| Go to a page | Navigate To | `POST /session/{id}/url` |
| Find an element | Find Element | `POST /session/{id}/element` |
| Click it | Element Click | `POST /session/{id}/element/{element id}/click` |
| Text of it | Get Element Text | `GET /session/{id}/element/{element id}/text` |
| Screenshot | Take Screenshot | `GET /session/{id}/screenshot` |

browser_bot never builds these URLs by hand — thirtyfour does — but
knowing the protocol shape is what makes the crate's API feel obvious:
`goto`, `query(...).click()`, `text()`, `screenshot` are thin wrappers
over those endpoints. This is the whole trick of WebDriver: Chrome only
listens to CDP, chromedriver translates, and any language with an HTTP
client can automate a browser.

### Connecting: attach vs. managed vs. external

`BrowserSession::connect` picks between three modes, in order:

1. **`WEBDRIVER_URL` set** → `WebDriver::new(&url, caps)`: plain HTTP
   client to a server you started yourself (e.g. `chromedriver
   --port=9515`). Thirtyfour also has `WebDriver::builder` for tuning
   request timeouts and the user agent.
2. **Attach mode** → if a TCP probe finds a listener on the Chrome
   remote-debugging address (default `127.0.0.1:9222`, override
   `CHROME_DEBUG_ADDRESS`), the bot attaches to the user's own Chrome:
   `ChromeCapabilities::set_debugger_address(addr)` sets
   `goog:chromeOptions.debuggerAddress`, which makes chromedriver attach
   to an already running browser instead of spawning one. The probe is
   std-only (`TcpStream::connect_timeout`, 100 ms) and a failed attach
   falls through with a notice — never an error.
3. **Default** → `WebDriver::managed(caps)`: thirtyfour downloads the
   chromedriver matching your installed Chrome, spawns it, and kills it
   when the session ends. This is why v0_01 has no install step.

Why attach mode exists: a bot-spawned Chrome starts with no cookies, so
sites like x.com need a fresh login (often blocked for automation
anyway). The user launches their own Chrome with
`--remote-debugging-port=9222 --user-data-dir=<dedicated profile>`,
logs in once, and the bot drives that window — login included. `quit()`
just detaches; the user's Chrome stays open (verified empirically).

In all cases, `DesiredCapabilities::chrome()` describes the browser we
want (headless would add a capability argument — see the roadmap).

### Windows and tabs

Chromium lists every tab as a separate window handle. The bot exposes
`window_handles()`, `current_url()`, `switch_to_window()`, and
`windows_with_urls()` — the last one enumerates handle→URL pairs by
switching to each window briefly and restoring the original, so the
user's visible window is never disturbed. Attach mode depends on this:
`run twitter-login` scans the handles for a twitter.com/x.com URL and
switches to it instead of navigating. One quirk worth knowing:
`new_tab()` does **not** make the new tab current, so code switches to
the new handle explicitly (the integration test documents this).

### Implicit waits

```rust
driver.set_implicit_wait_timeout(IMPLICIT_WAIT).await?;
```

This tells the *browser* to wait up to 10 seconds when asked to find an
element before reporting "no such element". Without it, `click` on a slow
page would fail instantly. With it, a missing selector fails after 10
seconds with a clean error. Trade-offs and alternatives (explicit waits
with conditions) are on the v0_02 roadmap.

### The query interface

`BrowserSession::element` is the shared lookup used by `click`, `type`,
`read`, and `prop`:

```rust
self.driver
    .query(By::Css(selector))
    .desc(&format!("element matching \"{selector}\""))
    .single()
    .await
```

- `query` starts a lookup that **polls** until the element appears or the
  implicit wait expires — no hand-written retry loops.
- `.desc(...)` names the target in error messages.
- `.single()` asks for exactly one element (there's a `.first()`, too).

`By::Css` is one of many locator strategies (id, class, xpath, ...).
CSS is the v0_01 default because it's expressive and well documented
([MDN CSS selectors](https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_selectors)).

### Quitting

`BrowserSession::quit` consumes `self` and calls `WebDriver::quit`, which
ends the session and closes Chrome. The doc comments in `main.rs` and the
crate docs explain why dropping without quitting is a trap: Rust has no
async destructors, so thirtyfour's synchronous drop would block the
executor and swallow errors. Making `quit` take `self` means the compiler
rejects using a closed session — ownership encodes the lifecycle.

## `src/subroutines.rs` — multi-step flows (`run`)

The single-step commands map to one WebDriver call each. A subroutine is
a small program: navigate, poll the page, wait for a human. `run
twitter-login` is the first one.

### The registry pattern

Subroutines are discovered through one table:

```rust
pub fn registry() -> &'static [(&'static str, &'static str)] {
    &[("twitter-login", "open Twitter and wait for the user to log in")]
}
```

`help` prints the table and `check_known` validates names against it, so
the help text can never drift from the dispatch in `execute`.
`check_known` is pure (returns `Result<(), String>`, no I/O) precisely so
it can be unit-tested without a browser session. Adding a subroutine =
one handler function + one registry row.

### Waiting for a human: poll until a condition

`wait_for_login` is the roadmap's "explicit wait" idea, hand-rolled:
every 2 seconds it runs two JavaScript probes in the page (via
`BrowserSession::eval_bool`, WebDriver's Execute Script endpoint) and
declares login when two consecutive polls show (a) real page text and
(b) none of the logged-out markers. The two-poll rule absorbs the
twitter.com → x.com redirect and loading windows; the text rule stops an
empty intermediate page from counting as a false positive. A fixed
deadline turns the loop into a timeout error instead of a hang.

The markers are **negative indicators** — elements that exist on the
logged-out login wall and disappear after login (verified against the
live x.com markup in Nov 2026: the wall is a "jetfuel" auth form with
`input[name="username_or_email"]`, a hidden `input[name="password"]`, a
Google-button iframe, and a mobile Sign-in link). Negative markers are
the practical choice because the logged-in page can't be inspected
without an account; the trade-off is that x.com markup changes are a
maintenance risk — documented in the README, with Ctrl+C and the
timeout as the escapes.

### Testability without accounts

`wait_for_login` takes the timeout as an argument and doesn't care which
page it's looking at — the caller navigates. The integration tests
exploit that: a `data:` URL whose login input a JS timer removes after
3 seconds plays the logged-out → logged-in transition, so detection,
already-logged-in, and timeout are all tested with no network and no
credentials (`tests/subroutines_integration.rs`). Same trick as the
main integration test, applied to a time-based flow.

### Cancellation

`run` is dispatched inside a `tokio::select!` that races the subroutine
against Ctrl+C. Whichever future completes first wins; the other is
dropped. Dropping the subroutine mid-WebDriver-call is safe — the next
command starts fresh. Without this race, Ctrl+C during a 10-minute wait
would only be noticed by the *outer* input loop, which isn't running
yet.

## Testing strategy

Three layers, in `cargo test`:

1. **Unit tests** (`repl.rs`, `subroutines.rs`, in-module): parsing,
   tokenizing, and the subroutine registry — no I/O, instant.
2. **Integration tests** (`tests/browser_integration.rs`,
   `tests/subroutines_integration.rs`): start a real Chrome and exercise
   the public library API against `data:` URLs (no network needed — the
   page is the URL). The subroutine tests fake the login flow with a JS
   timer. They *skip with a notice* when no browser can start, so
   `cargo test` passes on machines without Chrome or internet. Skipping
   (rather than failing) a test that depends on an environment is a
   judgment call: document loudly when you make it.
3. **Manual acceptance** (see README): the REPL itself, driven by a human
   at a real terminal.

## Conventions worth noticing

- **Doc comments everywhere** — every public item and every non-obvious
  decision is documented. The agent instructions require this; it's also
  how a codebase teaches.
- **`with_context` on every fallible step** — errors answer "what were we
  doing?" not just "what went wrong?"
- **Fail fast at startup, never crash at runtime** — `main` refuses to
  start without a browser, but a bad instruction is just a printed error.
- **Ownership as documentation** — `quit(self)` and `mem::take` show how
  Rust types encode intent that would be comments in other languages.

## Next steps for learning

1. Read `repl.rs` top to bottom, then its tests. Run
   `cargo test repl` and add a test for a case you invent.
2. Add a tiny command (e.g. `title` — `driver.title().await`). It touches
   exactly two places: `parse` and `dispatch`, plus a `BrowserSession`
   method. This is the smallest possible tour of the whole architecture.
3. Watch the traffic: start `chromedriver --port=9515` yourself, run
   browser_bot with `WEBDRIVER_URL` set, and point a browser or
   `curl -X POST localhost:9515/session` at it. The W3C spec linked above
   is the reference for what you'll see.
