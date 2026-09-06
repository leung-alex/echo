//! Bounded native perspective composition.
#[cfg(windows)]
pub(crate) mod bridge;
pub(crate) mod budget;
pub mod compositor;

mod offscreen;
