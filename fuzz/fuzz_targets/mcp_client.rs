#![no_main]
//! Responses from an untrusted external MCP server, read by the MCP client.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| soyuz_mcp::fuzzing::client_responses(data));
