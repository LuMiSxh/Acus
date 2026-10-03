//! Patches in the Codex `apply_patch` format plus `*** Replace Symbol: path#Sym`,
//! applied all-or-nothing with atomic writes.
mod apply;
mod parse;

pub use apply::{Applied, Change, Region, apply};
pub use parse::{Chunk, Op, parse};
