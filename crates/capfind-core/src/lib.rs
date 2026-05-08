//! # capfind-core
//!
//! The shared data model and on-disk index format for capfind.
//!
//! This crate has **no dependencies on other capfind crates**. Parsers produce
//! [`Capability`] values; the search layer consumes them; the CLI serialises
//! them to `.capfind/index.cfi`.
//!
//! See [`index`] for the file format, [`model`] for the data shapes.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod index;
pub mod model;

pub use error::{IndexError, Result};
pub use index::{load_index, read_header, write_index, IndexBody, IndexHeader, Posting, HEADER_LEN, INDEX_VERSION, MAGIC};
pub use model::{Capability, Field, FileStat, HttpInfo, Kind, Lang, RpcInfo, TermRef};
