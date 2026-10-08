//! `soyuz-mcp`: Soyuz and the Model Context Protocol (roadmap A3 + A4).
//!
//! **Server (A3).** Exposes the soyuz-ai tool catalog, resources
//! (`soyuz://location/current`, `soyuz://selection`, `soyuz://history`, `file://` inside
//! the client's folders) and prompts to MCP clients. Hand-rolled JSON-RPC 2.0, dual-era:
//! the stateless 2026-07-28 revision and the `initialize` handshake of ≤ 2025-11-25.
//! Transports:
//! * stdio — `soyuz --mcp` ([`cli`]): attaches to the running window over a local named
//!   pipe ([`pipe`]) so actions show in the UI, else serves headless ([`headless`]).
//! * Streamable HTTP on 127.0.0.1 ([`http`]) — off by default, bearer token per client,
//!   Host / Origin checks.
//!
//! Clients never get raw filesystem access: tools go through [`soyuz_ai::tools::dispatch`]
//! (schema, risk ceiling, privacy, scope) and write tools only produce plans; a plan runs
//! as a job after the user approves it in the window ([`server::Backend`]). Every call is
//! appended to `ai-audit.jsonl` ([`soyuz_ai::audit`]). See [`trust`] for the client model.
//!
//! **Client (A4).** [`client`] connects to external MCP servers (stdio / HTTP) and offers
//! their tools to the in-app assistant through [`client::ExternalTools`].
//!
//! Why not the official `rmcp` SDK (3.5, Apache-2.0, actively maintained): it is
//! tokio-only, while Soyuz is synchronous and needs custom transports (named pipe), a
//! per-request trust model and dual-era serving of the brand-new stateless revision. The
//! protocol surface used here is small enough to own.
#![deny(unsafe_code)]

pub mod cli;
pub mod client;
pub mod headless;
pub mod http;
pub mod install;
pub mod jobs;
pub mod jsonrpc;
pub mod pipe;
pub mod protocol;
pub mod server;
pub mod trust;

pub use server::{ApprovalDecision, ApprovalRequest, Backend, Mode, ProgressFn, Server, Session, Sink};
pub use trust::{ClientPolicy, Transport};
