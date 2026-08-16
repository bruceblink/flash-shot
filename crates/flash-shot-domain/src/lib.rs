//! Product value types and state machines with no GPUI or Windows API dependency.

#![cfg_attr(test, allow(clippy::expect_used, clippy::panic, clippy::unwrap_used))]

pub mod domain;

pub use domain::*;
