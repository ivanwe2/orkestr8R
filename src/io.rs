//! Writers that can be shared: the CLI's stdout and stderr are written by the command and by the
//! event sinks at the same time, and tests capture them into memory.

use std::io::Write;
use std::sync::{Arc, Mutex, PoisonError};

/// A cloneable handle to one writer, behind a mutex.
#[derive(Clone)]
pub struct SharedWriter(Arc<Mutex<Box<dyn Write + Send>>>);

impl SharedWriter {
    pub fn new(writer: impl Write + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Box::new(writer))))
    }

    pub fn stdout() -> Self {
        Self::new(std::io::stdout())
    }

    pub fn stderr() -> Self {
        Self::new(std::io::stderr())
    }

    /// A writer into memory, and the handle to read it back.
    pub fn capture() -> (Self, Captured) {
        let captured = Captured::default();
        (Self::new(CaptureWriter(captured.clone())), captured)
    }

    /// Writes `text` and flushes. Errors are ignored: a closed pipe must not crash `ork`.
    pub fn write_str(&self, text: &str) {
        let mut writer = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writer.write_all(text.as_bytes());
        let _ = writer.flush();
    }

    /// Writes `line` and a newline, then flushes.
    pub fn write_line(&self, line: &str) {
        self.write_str(&format!("{line}\n"));
    }
}

/// What a [`SharedWriter::capture`] writer received.
#[derive(Clone, Default)]
pub struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub fn text(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

struct CaptureWriter(Captured);

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut bytes = (self.0).0.lock().unwrap_or_else(PoisonError::into_inner);
        bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
