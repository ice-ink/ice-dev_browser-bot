# Setting up browser_bot on a new machine (v0_01)

A from-scratch guide for getting browser_bot running on a new system —
Windows, macOS, or Linux. It assumes programming fundamentals but not
Rust or WebDriver knowledge. If a step fails, see
[Troubleshooting](#troubleshooting).

The program's parts:

- **browser_bot** — a Rust program you run from a terminal; it types a
  prompt and carries out your instructions in a real Chrome window.
- **Chrome** — the browser that does the actual work. You install it.
- **chromedriver** — a small program that translates between browser_bot
  and Chrome. It is downloaded automatically on first run, so you never
  install it by hand.

## 1. Install the two prerequisites

**Rust.** Install via [rustup](https://rustup.rs) (the official
installer, all platforms). Any Rust 1.85 or newer works (the code uses
edition 2024); it was built and tested with 1.99. Check with:

```
rustc --version
```

**Chrome.** Install [Google Chrome](https://www.google.com/chrome/) the
normal way for your platform (Windows/macOS: the installer from the
site, or `brew install --cask google-chrome`; Debian/Ubuntu: the `.deb`
from the site or `apt install google-chrome-stable`).

## 2. Get the code and build it

Copy or clone the repository to the new machine, then enter the current
version directory. This repo keeps each version in its own `v0_0x`
directory; the current one is `v0_01`:

```
cd v0_01
cargo build
```

The first build downloads dependencies and takes a few minutes.

## 3. First run

```
cargo run
```

A Chrome window opens and you get a prompt:

```
browser_bot v0.1.0
Connecting to Chrome (first run may download chromedriver)...
Ready. Type `help` for the command list, `quit` to exit.
```

The first run downloads the chromedriver that matches your Chrome
version (this is why step 3 needs internet access). Try `open
https://example.com`, `read`, `quit`. You're set up — the rest of this
guide is the recommended extras.

## 4. Recommended: a dedicated browser profile (attach mode)

A fresh Chrome window started by the bot has no logins and is flagged as
automated, which sites like x.com can block. The workaround: you launch
Chrome yourself — once, with a special flag — into a profile that keeps
its logins. From then on the bot detects that Chrome and drives *your*
window instead of spawning its own.

**The flag** is the same on every OS; only the Chrome binary path and
profile directory differ:

```
<chrome-binary> --remote-debugging-port=9222 --no-first-run --user-data-dir=<profile-dir>
```

- `--remote-debugging-port=9222` — lets the bot attach (primary source:
  [Chrome DevTools remote debugging](https://developer.chrome.com/docs/devtools/remote-debugging)).
- `--no-first-run` — skips Chrome's welcome page, which otherwise delays
  the window you asked for.
- `--user-data-dir=<profile-dir>` — a directory *dedicated to the bot*.
  Never point it at your daily browsing profile: two Chromes on one
  profile lock each other out, and the bot would be driving your real
  session.

### Windows

Chrome binary: `C:\Program Files\Google\Chrome\Application\chrome.exe`.
Profile directory: e.g. `C:\browser-bot-chrome`.

From `cmd.exe`, the Run dialog (`Win+R`), or the *Target* field of a new
shortcut:

```
"C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222 --no-first-run --user-data-dir=C:\browser-bot-chrome
```

Creating a shortcut with this as its Target is the most convenient:
double-click it whenever you want the bot to use your profile.

### macOS

Chrome binary:
`/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`.
Profile directory: e.g. `~/browser-bot-chrome`.

From a terminal:

```
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --remote-debugging-port=9222 --no-first-run \
  --user-data-dir="$HOME/browser-bot-chrome"
```

Leave that terminal open (Ctrl+C later closes the Chrome it started).

### Linux

Chrome binary: `google-chrome` (package name varies: `google-chrome` on
Debian/Ubuntu, `google-chrome-stable` elsewhere — use whichever your
package provides). Chromium also works for attach mode.
Profile directory: e.g. `~/.browser-bot-chrome`.

```
google-chrome --remote-debugging-port=9222 --no-first-run \
  --user-data-dir="$HOME/.browser-bot-chrome" &
```

Note: sandboxed installs (notably snap packages) restrict custom profile
directories; if the launch fails, use the `.deb` package from
[google.com/chrome](https://www.google.com/chrome/).

### Use it (all platforms)

1. Launch Chrome with the command above, and in the window that opens,
   log into Twitter once. The profile remembers the login.
2. Run `cargo run` in `v0_01`. The startup line confirms the attach:

```
Attached to your Chrome at 127.0.0.1:9222 (it stays open when the bot exits).
```

3. Try `windows` (lists the windows the bot can drive) and then
   `run twitter-bookmarks <username> <password>` (reuses your login if
   present, otherwise fills the form, then opens bookmarks).

If your profile Chrome isn't running, the bot silently falls back to
spawning its own fresh Chrome — same commands, no login. If a different
program occupies port 9222, the bot prints a notice and falls back too.

## 5. Optional: environment variables

The bot reads two variables; both are optional and rarely needed.

| Variable | Effect | Default |
| --- | --- | --- |
| `CHROME_DEBUG_ADDRESS` | Attach to a remote-debugging Chrome on a different address/port | `127.0.0.1:9222` |
| `WEBDRIVER_URL` | Talk to a chromedriver you started yourself instead of the bot's managed one | (managed) |

Set them like any other variable:

- sh/bash/zsh: `export CHROME_DEBUG_ADDRESS=127.0.0.1:9223`
- PowerShell: `$env:CHROME_DEBUG_ADDRESS = "127.0.0.1:9223"`
- cmd.exe: `set CHROME_DEBUG_ADDRESS=127.0.0.1:9223`

## 6. Verify the install

```
cd v0_01
cargo test
```

Unit tests run everywhere, instantly. The live-browser tests open real
Chrome windows and **skip with a notice** on machines where no browser
can start, so `cargo test` should pass either way.

A manual smoke test, with or without the dedicated profile:

```
cargo run
browser-bot> open https://example.com
browser-bot> read
browser-bot> windows
browser-bot> run twitter-bookmarks <user> <password>
browser-bot> quit
```

## 7. Troubleshooting

| Symptom | Cause and fix |
| --- | --- |
| `could not start a managed Chrome session` | Chrome is missing, or the first-run chromedriver download needs internet. Install Chrome / check the connection. |
| Chrome's welcome page opens instead of your site | First run of a brand-new profile. Add `--no-first-run` (step 4). |
| `Profile is in use` / the launch opens in your daily Chrome | The profile directory is already in use (or you used your daily profile's directory). Use a dedicated directory. |
| Bot says nothing about attaching | Port 9222 isn't listening — launch the profile Chrome first. |
| `note: something is listening at ... but attaching failed` | Another program owns port 9222. Pick a free port and set `CHROME_DEBUG_ADDRESS`. |
| `no element matched "..."` | The page didn't show that selector within 10 s. Check the selector, or `wait` first. |
| `run twitter-bookmarks` never detects login / form fill fails | x.com may have redesigned its login flow. Ctrl+C, `screenshot login-fail.png`, then update the markers/selectors in `src/subroutines.rs` (see the README note). Prefer attach mode after one successful login. |

## 8. Where the documentation lives

- [../README.md](../README.md) — user guide and command reference.
- [architecture.md](architecture.md) — how the code works, for learners.
- [../CHANGES.md](../CHANGES.md) — what changed in each version.
- [roadmap-v0_02.md](roadmap-v0_02.md) — planned work for the next
  version.
