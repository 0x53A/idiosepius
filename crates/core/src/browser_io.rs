//! Synchronous in-memory files for the browser build.
//!
//! Turso's browser package uses asynchronous OPFS I/O in a worker. The rest of
//! idiosepius deliberately has a synchronous database façade, so the browser
//! build keeps the live SQLite files in memory and lets the app copy coherent
//! snapshots to OPFS between egui frames.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use turso_core::io::{FileId, FileSyncType};
use turso_core::{
    Buffer, Clock, Completion, File, IO, MonotonicInstant, OpenFlags, WallClockInstant,
};

pub(crate) const DB_NAME: &str = "study.db";
pub(crate) const WAL_NAME: &str = "study.db-wal";

pub(crate) struct BrowserIo {
    files: Mutex<HashMap<String, Arc<BrowserFile>>>,
    generation: Arc<AtomicU64>,
}

impl BrowserIo {
    pub(crate) fn new(database: Vec<u8>, wal: Vec<u8>) -> Self {
        let generation = Arc::new(AtomicU64::new(0));
        let mut files = HashMap::new();
        if !database.is_empty() {
            files.insert(
                DB_NAME.to_owned(),
                Arc::new(BrowserFile::new(database, generation.clone())),
            );
        }
        if !wal.is_empty() {
            files.insert(
                WAL_NAME.to_owned(),
                Arc::new(BrowserFile::new(wal, generation.clone())),
            );
        }
        Self {
            files: Mutex::new(files),
            generation,
        }
    }

    pub(crate) fn snapshot(&self) -> BrowserSnapshot {
        let files = self.files.lock().expect("browser database files poisoned");
        BrowserSnapshot {
            generation: self.generation.load(Ordering::Acquire),
            database: files
                .get(DB_NAME)
                .map(|file| file.bytes())
                .unwrap_or_default(),
            wal: files
                .get(WAL_NAME)
                .map(|file| file.bytes())
                .unwrap_or_default(),
        }
    }
}

pub struct BrowserSnapshot {
    pub generation: u64,
    pub database: Vec<u8>,
    pub wal: Vec<u8>,
}

impl Clock for BrowserIo {
    fn current_time_monotonic(&self) -> MonotonicInstant {
        static EPOCH: LazyLock<web_time::Instant> = LazyLock::new(web_time::Instant::now);
        MonotonicInstant::from_nanos(EPOCH.elapsed().as_nanos())
    }

    fn current_time_wall_clock(&self) -> WallClockInstant {
        let duration = web_time::SystemTime::now()
            .duration_since(web_time::UNIX_EPOCH)
            .unwrap_or_default();
        WallClockInstant {
            secs: duration.as_secs() as i64,
            micros: duration.subsec_micros(),
        }
    }
}

impl IO for BrowserIo {
    fn open_file(
        &self,
        path: &str,
        flags: OpenFlags,
        _direct: bool,
    ) -> turso_core::Result<Arc<dyn File>> {
        let mut files = self.files.lock().expect("browser database files poisoned");
        if let Some(file) = files.get(path) {
            return Ok(file.clone());
        }
        if !flags.contains(OpenFlags::Create) {
            return Err(
                turso_core::CompletionError::IOError(std::io::ErrorKind::NotFound, "open").into(),
            );
        }
        let file = Arc::new(BrowserFile::new(Vec::new(), self.generation.clone()));
        files.insert(path.to_owned(), file.clone());
        Ok(file)
    }

    fn remove_file(&self, path: &str) -> turso_core::Result<()> {
        self.files
            .lock()
            .expect("browser database files poisoned")
            .remove(path);
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn file_id(&self, path: &str) -> turso_core::Result<FileId> {
        Ok(FileId::from_path_hash(path))
    }

    fn supports_shared_wal_coordination(&self) -> bool {
        false
    }
}

struct BrowserFile {
    data: Mutex<Vec<u8>>,
    generation: Arc<AtomicU64>,
}

impl BrowserFile {
    fn new(data: Vec<u8>, generation: Arc<AtomicU64>) -> Self {
        Self {
            data: Mutex::new(data),
            generation,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        self.data
            .lock()
            .expect("browser database file poisoned")
            .clone()
    }

    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
    }
}

impl File for BrowserFile {
    fn lock_file(&self, _exclusive: bool) -> turso_core::Result<()> {
        Ok(())
    }

    fn unlock_file(&self) -> turso_core::Result<()> {
        Ok(())
    }

    fn pread(&self, pos: u64, completion: Completion) -> turso_core::Result<Completion> {
        let data = self.data.lock().expect("browser database file poisoned");
        let pos = pos as usize;
        let count = if pos >= data.len() {
            0
        } else {
            let count = completion.as_read().buf().len().min(data.len() - pos);
            completion.as_read().buf().as_mut_slice()[..count]
                .copy_from_slice(&data[pos..pos + count]);
            count
        };
        completion.complete(count as i32);
        Ok(completion)
    }

    fn pwrite(
        &self,
        pos: u64,
        buffer: Arc<Buffer>,
        completion: Completion,
    ) -> turso_core::Result<Completion> {
        let mut data = self.data.lock().expect("browser database file poisoned");
        let pos = pos as usize;
        let end = pos.saturating_add(buffer.len());
        if data.len() < end {
            data.resize(end, 0);
        }
        data[pos..end].copy_from_slice(buffer.as_slice());
        drop(data);
        self.changed();
        completion.complete(buffer.len() as i32);
        Ok(completion)
    }

    fn sync(
        &self,
        completion: Completion,
        _sync_type: FileSyncType,
    ) -> turso_core::Result<Completion> {
        completion.complete(0);
        Ok(completion)
    }

    fn size(&self) -> turso_core::Result<u64> {
        Ok(self
            .data
            .lock()
            .expect("browser database file poisoned")
            .len() as u64)
    }

    fn truncate(&self, len: u64, completion: Completion) -> turso_core::Result<Completion> {
        self.data
            .lock()
            .expect("browser database file poisoned")
            .resize(len as usize, 0);
        self.changed();
        completion.complete(0);
        Ok(completion)
    }
}
