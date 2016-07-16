#![deny(missing_docs)]

//! A collection of safe low-latency low-level wait-free concurrent queues.
//!
//! This crate is a collection of concurrent queues for various scenarios:
//!
//! - single producer single consumer
//! - single producer multiple consumers
//! - multiple producers single consumer
//! - multiple producers multiple consumers
//!
//! It also offers a unified interface for all those queues, which facilitates
//! usage and refactoring.
//!
//! # Warning
//!
//! Although these queues themselves are wait-free, many users will probably
//! wish to implement higher-level interfaces (for example, by busy-waiting
//! until an operation succeeds). Those higher-level interface might not be
//! wait-free, they might also suffer from starvation, ...

pub mod interface;
pub mod spsc;
pub mod spmc;
pub mod mpsc;
pub mod mpmc;

mod common;
