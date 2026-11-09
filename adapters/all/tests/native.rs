//! Each native reader reports a failed vendor check as its own rule says:
//! the readers side by side, as the map runs them.

use serde_json::Value;
use xenonet_adapter::reader::files::StdFiles;
use xenonet_adapter::reader::native::{Checked, NativeReader, Threw, Vendor};
use xenonet_sources::native::VENDOR_FAILED;

/// A vendor reader that can't be got (`got` false), or whose every check
/// throws.
struct Broken {
    got: bool,
}

impl Vendor for Broken {
    fn ready(&mut self) -> Result<(), Threw> {
        if self.got {
            Ok(())
        } else {
            Err(Threw)
        }
    }
    fn missing(&mut self, _: &str, _: Option<&str>) -> Result<(), Threw> {
        Ok(())
    }
    fn check(&mut self, _: &Value, _: Option<&str>) -> Result<Checked, Threw> {
        Err(Threw)
    }
}

#[test]
fn each_reader_reports_a_failed_vendor_check_as_its_rule_says() {
    let dir = std::env::temp_dir().join(format!("xenonet-native-vendor-rs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let now = xenonet_base::time::parse_iso("2026-10-16T12:00:00.000Z").unwrap();
    let read = |r: &mut dyn NativeReader, path: &str, got: bool| r.read(path, now, &mut StdFiles, &mut Broken { got });
    let failed = Err(VENDOR_FAILED.to_string());

    // Gemini CLI and Copilot CLI get the reader before anything else, and
    // pass over a file whose check threw.
    let readers: [fn() -> Box<dyn NativeReader>; 2] = [|| Box::new(xenonet_gemini_cli::reader::GeminiCli::new()), || Box::new(xenonet_copilot::reader::CopilotCli::new())];
    for make in readers {
        let mut r = make();
        assert_eq!(read(r.as_mut(), &at("none"), false), failed);
        assert_eq!(r.source_state(), "unread", "nothing was read");
        assert_eq!(read(r.as_mut(), &at("none"), true), Ok(()));
    }

    // Cursor gets it when its file changed, and a check that threw is
    // reported too.
    let events = at("cursor-hooks.ndjson");
    std::fs::write(&events, "{\"hook_event_name\":\"afterFileEdit\",\"conversation_id\":\"6b0e5f1c-1111\",\"at\":\"2026-10-16T12:00:00.000Z\"}\n").unwrap();
    let mut c = xenonet_cursor::reader::Cursor::new();
    assert_eq!(read(&mut c, &at("none.ndjson"), false), Ok(()), "a missing file is nothing to parse");
    assert_eq!(read(&mut c, &events, false), failed);
    assert_eq!(c.source_state(), "ok");
    let mut c = xenonet_cursor::reader::Cursor::new();
    assert_eq!(read(&mut c, &events, true), failed, "its check threw");

    // OpenCode gets it once its database has opened; a check that threw
    // leaves the database unreadable.
    let db = at("opencode.db");
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(
            "CREATE TABLE session (id text, directory text, time_created integer, time_updated integer); CREATE TABLE part (session_id text, time_created integer, data text);",
        )
        .unwrap();
    let mut o = xenonet_opencode::reader::OpenCode::new();
    assert_eq!(read(&mut o, &at("none.db"), false), Ok(()), "a missing database is nothing to open");
    assert_eq!(read(&mut o, &db, false), failed);
    assert_eq!(read(&mut o, &db, true), Ok(()));
    assert_eq!(o.source_state(), "unreadable");
    std::fs::remove_dir_all(&dir).unwrap();
}
