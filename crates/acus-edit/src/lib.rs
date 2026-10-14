//! Patches with `*** ` directives, SEARCH/REPLACE blocks and `*** Replace Symbol: path#Sym`,
//! applied all-or-nothing with atomic writes.
mod apply;
mod parse;

pub use apply::{Applied, Change, Region, apply};
pub use parse::{Chunk, InsertAt, Op, parse};
