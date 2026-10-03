//! Language detection, tree-sitter outlines and chainable addresses.
mod address;
mod lang;
mod outline;
mod rules;

pub use address::{Address, Resolve, Target, resolve};
pub use lang::Lang;
pub use outline::{Outline, Symbol, outline};
