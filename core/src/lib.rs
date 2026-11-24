//! Setu's core: the rules, the compiler that turns them into Syncthing folders, and the code
//! that applies them to this computer's Syncthing. The desktop app and the `setu` command both
//! use it.

pub mod accuracy;
pub mod apply;
pub mod compile;
pub mod control;
pub mod helper;
pub mod hub;
pub mod index;
pub mod local;
pub mod mesh;
pub mod migrate;
pub mod pipe;
pub mod preview;
pub mod progress;
pub mod rules;
pub mod session;
pub mod syncthing;
pub mod tidy;
pub mod trust;
pub mod watch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("rules: {0}")]
    Rules(String),
    #[error("Syncthing: {0}")]
    Syncthing(String),
    #[error("setup: {0}")]
    Setup(String),
    #[error("Brahma: {0}")]
    Hub(String),
    #[error("{}", "Enter your recovery key to unlock your synced data")]
    RecoveryKeyNeeded,
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
