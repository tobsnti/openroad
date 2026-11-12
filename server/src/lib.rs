//! The OpenRoad game server (ADR 0013).
//!
//! Idea: one owner for the whole world. A connection only frames bytes and
//! judges them strictly ([`inbound`]); the [`world`] turns the requests it
//! accepts into answers, as the single writer of all game state; and every
//! session drains its answers from a bounded queue ([`outbox`]), so a peer
//! that cannot keep up is cut off instead of buffered without end. What the
//! server does with each client opcode is one table ([`registry`]). Tests
//! drive the same world in-process, without sockets, one step at a time
//! ([`tick`]).

pub mod inbound;
pub mod outbox;
pub mod registry;
pub mod store;
pub mod tick;
pub mod wirelog;
pub mod world;
