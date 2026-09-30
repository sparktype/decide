pub mod backend;
pub mod daemon;
pub mod local;
pub mod mcp;
pub mod protocol;
pub mod typesafe;

pub use backend::{decide, decide_many, Backend, Env};
pub use protocol::{DecideManyResult, DecideResult};
