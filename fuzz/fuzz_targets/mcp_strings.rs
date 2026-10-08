#![no_main]
//! MCP string parsers: `file://` URIs, encoded header values, client names, prompt arguments.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| soyuz_mcp::fuzzing::strings(data));
