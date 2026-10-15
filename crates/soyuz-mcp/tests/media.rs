//! The media tools over MCP (Phase V): listed from the catalog with the right risk hints,
//! refused like other write tools for pending clients, argument errors as tool errors,
//! and (with the FFmpeg libraries) a trusted client's `media_frame` running as a job on
//! the headless engine, audited.

mod common;

use std::path::PathBuf;

use common::*;
use serde_json::{Value, json};
use soyuz_ai::tools::MediaAccess;
use soyuz_mcp::Server;
use soyuz_mcp::headless::HeadlessBackend;

const MEDIA_TOOLS: [&str; 5] = ["media_info", "media_convert", "media_extract_audio", "media_trim", "media_frame"];

fn listed(c: &mut Conn) -> Vec<Value> {
    c.call("tools/list", json!({}))["result"]["tools"].as_array().unwrap().clone()
}

#[test]
fn media_tools_are_listed_with_their_risk() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server.clone(), Some("tester"));
    let tools = listed(&mut c);
    for name in MEDIA_TOOLS {
        let t = tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name} listed"));
        assert_eq!(t["annotations"]["readOnlyHint"], name == "media_info", "{name}");
        assert_eq!(t["annotations"]["destructiveHint"], false, "{name}");
        let props = &t["inputSchema"]["properties"];
        assert!(props.get("path").is_some(), "{name}");
        assert_eq!(props.get("dry_run").is_some(), name != "media_info", "{name}");
    }
    // A pending client sees media_info only.
    let mut c = Conn::new(server, Some("Fresh Agent"));
    let names: Vec<String> = listed(&mut c).iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(names.contains(&"media_info".to_string()));
    assert!(!names.iter().any(|n| n == "media_convert" || n == "media_frame"));
    let r = c.tool("media_frame", json!({"path": fx.root.join("hello.txt"), "at": 0}));
    assert!(is_error(&r) && text_of(&r).contains("waiting for approval"), "{}", text_of(&r));
}

#[test]
fn media_tool_errors_are_tool_errors() {
    let fx = Fixture::new();
    let (server, _) = fx.headless(fx.ai());
    let mut c = Conn::new(server, Some("tester"));
    let hello = fx.root.join("hello.txt");
    // Schema: unknown preset.
    let r = c.tool("media_convert", json!({"path": hello, "preset": "x264"}));
    assert!(is_error(&r), "{r}");
    // Planner: an empty trim window.
    let r = c.tool("media_trim", json!({"path": hello, "start": 4, "end": 1, "dry_run": true}));
    assert!(is_error(&r) && text_of(&r).contains("after start"), "{}", text_of(&r));
    // Not a media file (or no libraries in this process): an error, never a panic.
    let r = c.tool("media_info", json!({"path": hello}));
    assert!(is_error(&r), "{r}");
    // Outside the scope.
    let outside = fx.dir.path().join("x.mkv");
    let r = c.tool("media_convert", json!({"path": hello, "preset": "mkv-copy", "dest": outside}));
    assert!(is_error(&r) && text_of(&r).contains("outside"), "{}", text_of(&r));
}

fn media_dirs() -> Option<Vec<PathBuf>> {
    let strict = std::env::var("SOYUZ_MEDIA_TESTS").is_ok_and(|v| v == "1");
    let dirs = match std::env::var_os(soyuz_media::DIR_ENV) {
        Some(d) => vec![PathBuf::from(d)],
        None => soyuz_media::search_dirs("", &std::env::temp_dir().join("soyuz-mcp-media-tests-config")),
    };
    match soyuz_media::Runtime::get(&dirs) {
        Ok(_) => Some(dirs),
        Err(e) => {
            assert!(!strict, "SOYUZ_MEDIA_TESTS=1 but no FFmpeg runtime loads: {e}");
            eprintln!("skipped: no FFmpeg runtime ({e})");
            None
        }
    }
}

#[test]
fn a_trusted_client_saves_a_frame_as_a_job() {
    let Some(dirs) = media_dirs() else { return };
    let fx = Fixture::new();
    let frame = soyuz_media::fixtures::jpeg_solid(32, 24, [20, 20, 200]);
    let frames: Vec<Vec<u8>> = (0..10).map(|_| frame.clone()).collect();
    let clip = fx.root.join("clip.avi");
    std::fs::write(&clip, soyuz_media::fixtures::avi(32, 24, 5, soyuz_media::fixtures::AviCodec::Mjpeg, &frames))
        .unwrap();
    let media = MediaAccess { dirs, ..MediaAccess::default() };
    let backend = HeadlessBackend::with_media(fx.ai(), media, Some(fx.config.clone()), None, fx.audit.clone());
    let server = Server::new(backend.clone());
    let mut c = Conn::new(server.clone(), Some("tester"));
    let r = c.tool("media_info", json!({"path": clip}));
    assert!(!is_error(&r) && text_of(&r).contains("mjpeg"), "{}", text_of(&r));
    // Dry run: the plan names the output; nothing is written.
    let r = c.tool("media_frame", json!({"path": clip, "at": 0.5, "dry_run": true}));
    assert!(!is_error(&r) && text_of(&r).contains("clip.png"), "{}", text_of(&r));
    assert!(!fx.root.join("clip.png").exists());
    // A client without approval runs it as a job.
    let mut c = Conn::new(server, Some("trusted"));
    let r = c.tool("media_frame", json!({"path": clip, "at": 0.5}));
    assert!(!is_error(&r), "{}", text_of(&r));
    assert!(text_of(&r).contains("\"state\":\"done\""), "{}", text_of(&r));
    assert!(std::fs::read(fx.root.join("clip.png")).unwrap().starts_with(b"\x89PNG"));
    let audit = audit_lines(&fx.audit);
    assert!(audit.iter().any(|r| r.tool == "media_frame" && r.outcome == "ok" && r.job_id.is_some()));
    backend.engine().shutdown(std::time::Duration::from_secs(5));
}
