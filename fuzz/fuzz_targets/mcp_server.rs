#![no_main]
//! JSON-RPC / MCP messages from an untrusted MCP client (stdio, named pipe, HTTP body).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| soyuz_mcp::fuzzing::server_messages(data));
