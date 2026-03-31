use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::thread::{self, JoinHandle};

use crossbeam_channel::Sender;
use serde_json::Value;

use super::pipewire_discovery::{Snapshot, parse_pw_dump};

/// Events delivered from the pw-dump monitor reader thread to the core loop.
#[derive(Debug)]
pub(crate) enum PwMonitorEvent {
    /// First complete JSON array parsed into a full Snapshot.
    InitialSnapshot(Snapshot),
    /// Subsequent JSON arrays containing changed PipeWire objects.
    ObjectsChanged(Vec<Value>),
    /// The pw-dump process exited or stdout closed.
    ProcessDied(String),
}

/// Extracts complete JSON arrays from a byte stream using bracket-depth tracking.
///
/// Reads character-by-character, tracking `[`/`]` nesting depth while respecting
/// JSON string literals (quoted regions with backslash escape awareness).
/// When depth returns to 0 after a top-level `[`, the accumulated buffer is
/// one complete JSON array.
///
/// Returns `None` on EOF (reader exhausted). Returns `Some(Err(...))` on parse failure.
/// Returns `Some(Ok(values))` for each complete JSON array.
fn read_next_json_array(reader: &mut impl Read) -> Option<Result<Vec<Value>, String>> {
    let mut buf = Vec::with_capacity(64 * 1024);
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut seen_open = false;
    let mut byte = [0u8; 1];

    loop {
        match reader.read(&mut byte) {
            Ok(0) => {
                // EOF
                return if seen_open {
                    Some(Err("unexpected EOF inside JSON array".to_string()))
                } else {
                    None // clean EOF, no partial data
                };
            }
            Ok(_) => {
                let ch = byte[0];
                buf.push(ch);

                if escape_next {
                    escape_next = false;
                    continue;
                }

                if in_string {
                    match ch {
                        b'\\' => escape_next = true,
                        b'"' => in_string = false,
                        _ => {}
                    }
                    continue;
                }

                match ch {
                    b'"' => in_string = true,
                    b'[' => {
                        depth += 1;
                        seen_open = true;
                    }
                    b']' => {
                        depth -= 1;
                        if depth == 0 && seen_open {
                            // Complete JSON array
                            let raw = String::from_utf8_lossy(&buf);
                            return match serde_json::from_str::<Vec<Value>>(&raw) {
                                Ok(values) => Some(Ok(values)),
                                Err(e) => Some(Err(format!("JSON parse error: {e}"))),
                            };
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                return Some(Err(format!("read error: {e}")));
            }
        }
    }
}

/// Manages a persistent `pw-dump --monitor` child process.
pub(crate) struct PwMonitor {
    child: Child,
    reader_thread: Option<JoinHandle<()>>,
}

impl PwMonitor {
    /// Spawn `pw-dump --monitor`, start reader thread.
    ///
    /// The first complete JSON array is parsed via `parse_pw_dump` and sent as
    /// `InitialSnapshot`. Subsequent arrays are sent as `ObjectsChanged`.
    /// If the process exits or stdout closes, sends `ProcessDied`.
    pub fn spawn(
        hidden_outputs: &[&str],
        hidden_inputs: &[&str],
        tx: Sender<PwMonitorEvent>,
    ) -> Result<Self, String> {
        let mut child = Command::new("pw-dump")
            .arg("--monitor")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn pw-dump --monitor: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "pw-dump stdout not captured".to_string())?;

        // Clone filter lists for the reader thread (owned Strings)
        let hidden_out: Vec<String> = hidden_outputs.iter().map(|s| s.to_string()).collect();
        let hidden_in: Vec<String> = hidden_inputs.iter().map(|s| s.to_string()).collect();

        let reader_thread = thread::Builder::new()
            .name("pw-monitor-reader".to_string())
            .spawn(move || {
                Self::reader_loop(stdout, &hidden_out, &hidden_in, &tx);
            })
            .map_err(|e| format!("failed to spawn reader thread: {e}"))?;

        Ok(Self {
            child,
            reader_thread: Some(reader_thread),
        })
    }

    fn reader_loop(
        stdout: impl Read,
        hidden_outputs: &[String],
        hidden_inputs: &[String],
        tx: &Sender<PwMonitorEvent>,
    ) {
        let mut reader = std::io::BufReader::new(stdout);
        let mut is_first = true;

        loop {
            match read_next_json_array(&mut reader) {
                None => {
                    // Clean EOF
                    let _ = tx.send(PwMonitorEvent::ProcessDied(
                        "pw-dump process ended (EOF)".to_string(),
                    ));
                    return;
                }
                Some(Err(e)) => {
                    let _ = tx.send(PwMonitorEvent::ProcessDied(e));
                    return;
                }
                Some(Ok(values)) => {
                    if is_first {
                        is_first = false;
                        // Serialize back to string for parse_pw_dump (which expects &str)
                        let raw = serde_json::to_string(&values).unwrap_or_default();
                        let ho: Vec<&str> = hidden_outputs.iter().map(|s| s.as_str()).collect();
                        let hi: Vec<&str> = hidden_inputs.iter().map(|s| s.as_str()).collect();
                        match parse_pw_dump(&raw, &ho, &hi) {
                            Ok(snapshot) => {
                                let _ = tx.send(PwMonitorEvent::InitialSnapshot(snapshot));
                            }
                            Err(e) => {
                                let _ = tx.send(PwMonitorEvent::ProcessDied(format!(
                                    "failed to parse initial snapshot: {e}"
                                )));
                                return;
                            }
                        }
                    } else {
                        let _ = tx.send(PwMonitorEvent::ObjectsChanged(values));
                    }
                }
            }
        }
    }

    /// Kill the child process and join the reader thread.
    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parse_single_json_array() {
        let input = b"[{\"id\": 1}, {\"id\": 2}]";
        let mut reader = Cursor::new(input);
        let result = read_next_json_array(&mut reader);
        let values = result.unwrap().unwrap();
        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["id"], 1);
        assert_eq!(values[1]["id"], 2);
    }

    #[test]
    fn parse_two_consecutive_arrays() {
        let input = b"[{\"id\": 1}]\n[{\"id\": 2}, {\"id\": 3}]";
        let mut reader = Cursor::new(input);

        let first = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0]["id"], 1);

        let second = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0]["id"], 2);
    }

    #[test]
    fn brackets_inside_strings_are_ignored() {
        let input = b"[{\"name\": \"test[0]\", \"id\": 1}]";
        let mut reader = Cursor::new(input);
        let values = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["name"], "test[0]");
    }

    #[test]
    fn escaped_quotes_inside_strings_handled() {
        let input = b"[{\"name\": \"he said \\\"hello\\\"\", \"id\": 1}]";
        let mut reader = Cursor::new(input);
        let values = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["id"], 1);
    }

    #[test]
    fn nested_arrays_tracked_correctly() {
        let input = b"[{\"ids\": [1, 2, 3]}, {\"ids\": [4]}]";
        let mut reader = Cursor::new(input);
        let values = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn eof_with_no_data_returns_none() {
        let input = b"";
        let mut reader = Cursor::new(input);
        assert!(read_next_json_array(&mut reader).is_none());
    }

    #[test]
    fn eof_mid_array_returns_error() {
        let input = b"[{\"id\": 1";
        let mut reader = Cursor::new(input);
        let result = read_next_json_array(&mut reader);
        assert!(result.unwrap().is_err());
    }

    #[test]
    fn whitespace_between_arrays_is_skipped() {
        let input = b"  \n\t  [{\"id\": 1}]  \n  [{\"id\": 2}]";
        let mut reader = Cursor::new(input);

        let first = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(first[0]["id"], 1);

        let second = read_next_json_array(&mut reader).unwrap().unwrap();
        assert_eq!(second[0]["id"], 2);
    }

    #[test]
    fn reader_loop_sends_initial_snapshot_then_changes() {
        // Simulate pw-dump output: first array = full state, second = incremental
        let first_array = serde_json::json!([
            {
                "id": 50,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": {
                        "media.class": "Audio/Sink",
                        "node.name": "test-sink",
                        "node.nick": "Test Sink",
                        "object.serial": "50"
                    }
                }
            }
        ]);
        let second_array = serde_json::json!([
            {
                "id": 50,
                "type": "PipeWire:Interface:Node",
                "info": {
                    "props": {
                        "media.class": "Audio/Sink",
                        "node.name": "test-sink",
                        "node.nick": "Test Sink Changed",
                        "object.serial": "50"
                    }
                }
            }
        ]);

        let input = format!("{}\n{}", first_array, second_array);
        let reader = Cursor::new(input.into_bytes());

        let (tx, rx) = crossbeam_channel::unbounded();
        let hidden_out: Vec<String> = vec![];
        let hidden_in: Vec<String> = vec![];

        PwMonitor::reader_loop(reader, &hidden_out, &hidden_in, &tx);

        // Should receive: InitialSnapshot, ObjectsChanged, ProcessDied(EOF)
        let ev1 = rx.recv().unwrap();
        assert!(
            matches!(ev1, PwMonitorEvent::InitialSnapshot(_)),
            "expected InitialSnapshot, got {ev1:?}"
        );

        let ev2 = rx.recv().unwrap();
        assert!(
            matches!(ev2, PwMonitorEvent::ObjectsChanged(_)),
            "expected ObjectsChanged, got {ev2:?}"
        );

        let ev3 = rx.recv().unwrap();
        assert!(
            matches!(ev3, PwMonitorEvent::ProcessDied(_)),
            "expected ProcessDied, got {ev3:?}"
        );
    }
}
