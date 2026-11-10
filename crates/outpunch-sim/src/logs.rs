//! Captured output. Each component's lines go to `<name>.log`; every line also goes to `all.log`
//! and to the simulator's stdout, interleaved in the order received. Each line is prefixed with
//! the time it was captured (`+12345ms`, since the simulator started) so they line up; the rest of
//! the line is exactly what the component wrote.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Clone)]
pub struct Logs {
    started: Instant,
    all: Arc<Mutex<File>>,
}

#[derive(Clone)]
pub struct Log {
    name: &'static str,
    file: Arc<Mutex<File>>,
    logs: Logs,
}

impl Logs {
    pub fn create(dir: &Path, started: Instant) -> std::io::Result<Self> {
        Ok(Self {
            started,
            all: Arc::new(Mutex::new(File::create(dir.join("all.log"))?)),
        })
    }

    pub fn component(&self, dir: &Path, name: &'static str) -> std::io::Result<Log> {
        Ok(Log {
            name,
            file: Arc::new(Mutex::new(File::create(dir.join(format!("{name}.log")))?)),
            logs: self.clone(),
        })
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }
}

impl Log {
    /// Writes the line to this component's file and to `all.log`, and prints it to the
    /// simulator's stdout so the run can be watched live.
    pub fn line(&self, text: &str) {
        let at = self.logs.elapsed_ms();
        let merged = format!("+{at:>7}ms {:<7} | {text}", self.name);
        let _ = writeln!(lock(&self.file), "+{at:>7}ms {text}");
        let _ = writeln!(lock(&self.logs.all), "{merged}");
        println!("{merged}");
    }
}

fn lock(file: &Mutex<File>) -> std::sync::MutexGuard<'_, File> {
    file.lock().unwrap_or_else(|e| e.into_inner())
}
