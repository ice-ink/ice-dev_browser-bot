//! browser_bot — a terminal-controlled browser agent.
//!
//! The user types short instructions at a prompt ("open https://example.com",
//! "click #submit", ...) and this program carries them out in a real Chrome
//! window. Under the hood every instruction becomes one or more WebDriver
//! commands: small HTTP requests sent to chromedriver, which relays them to
//! Chrome. See [`browser`] for the browser plumbing, [`repl`] for the
//! terminal side, and [`subroutines`] for the multi-step `run` flows.
//!
//! ## Crate layout
//!
//! This crate has both a library target (`src/lib.rs`) and a binary target
//! (`src/main.rs`). The library holds all the logic; `main` is a thin
//! wrapper. Splitting it this way lets the integration tests in `tests/`
//! import `browser_bot::browser` — Rust binaries can't be imported, but
//! libraries can. It also keeps the door open for future frontends (for
//! example a script/batch mode) that reuse the same library.

pub mod browser;
pub mod repl;
pub mod subroutines;
