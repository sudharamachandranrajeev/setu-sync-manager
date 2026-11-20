//! Setu's core: the rules, the compiler that turns them into Syncthing folders, and the code
//! that applies them to this computer's Syncthing. The desktop app and the `setu` command both
//! use it.

pub mod apply;
pub mod compile;
pub mod control;
pub mod index;
pub mod local;
pub mod rules;
pub mod session;
pub mod syncthing;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("rules: {0}")]
    Rules(String),
    #[error("Syncthing: {0}")]
    Syncthing(String),
    #[error("setup: {0}")]
    Setup(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
