# Roadmap — v0_02

Documented when v0_01 finished, as the agent instructions require. The
theme for v0_02: make the agent more comfortable to drive and a step more
autonomous.

## Delivered since this roadmap was written (v0_01 update)

The **subroutines category** arrived ahead of v0_02, still in v0_01:
`run <name>` with the first subroutine `run twitter-login` (open
Twitter, block the prompt, poll until the user has logged in), plus
`BrowserSession::eval_bool` (the JS-execution groundwork for the `js`
command below) and the first explicit-wait machinery (the poll loop in
`wait_for_login`). Items 5 (`js` groundwork) and 6 (subroutine side) are
partially done; the REPL-facing `wait-for` command remains open.

## Candidate features (priority order)

1. **Script/batch mode** — run a list of instructions from a file
   (`browser_bot run tasks.txt`), reusing the same parser and
   `BrowserSession` as the REPL. The library/binary split in v0_01 was
   designed for exactly this.
2. **Config file** — default WebDriver URL, wait timeouts, screenshot
   directory, instead of environment variables only.
3. **Headless flag** — `--headless` capability for running without a
   visible window (useful for servers and for script mode).
4. **Command history and line editing** — e.g. the
   [rustyline](https://docs.rs/rustyline) crate: arrows to recall and edit
   previous lines.
5. **More commands:**
   - `value <css-selector>` — read an input's value (closes the v0_01
     `read` gap).
   - `submit` / `keys` — form submission and key combinations (Enter,
     Tab, shortcuts).
   - `js <expression>` — evaluate JavaScript in the page.
   - `title`, `url` — quick page facts.
6. **Explicit waits** — `wait-for <selector>` (wait until an element
   appears) instead of fixed-duration `wait`, using thirtyfour's
   wait predicates. (Subroutine-side polling already exists in
   `wait_for_login`; this item is about exposing it as a REPL command.)
7. **Error recovery** — retry on stale-element errors after navigation,
   and a clearer message when a click is intercepted.
8. **More subroutines** — build on the registry: the pattern is one
   handler + one row in `src/subroutines.rs`.

## Ideas beyond v0_02

- **LLM autonomy**: interpret a free-form goal ("find the price on this
  page and save it to a file") into a command sequence. The v0_01 command
  model is the action layer such a planner would emit.
- Multiple tabs/windows, cookies, and frames.
- A `--trace` mode that logs every WebDriver request for learning.

## Acceptance for v0_02

To be finalized when v0_02 starts; at minimum the same bar as v0_01:
user guide and architecture docs updated, tests passing, a v0_02
changelog entry, and a roadmap-v0_03 written when v0_02 finishes.
