//! A template package. Its methods are not part of [`registry`](super::registry), because
//! Last.fm has no such methods; tests register them by hand.

mod echo;

pub use echo::Echo;
