//! Language detection, tree-sitter outlines and chainable addresses.
mod lang;
mod outline;
mod rules;

pub use lang::Lang;
pub use outline::{Outline, Symbol, outline};
