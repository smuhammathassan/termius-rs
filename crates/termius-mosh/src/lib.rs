//! Mosh (mobile shell) support. Stub for now: the original uses a native @termius/mosh addon; Rust port shells out to a system `mosh`/`mosh-client` or is deferred. Marked experimental.
//!
//! Ported from the Termius Electron app (see REA analysis + recovered sources).

pub mod error;
