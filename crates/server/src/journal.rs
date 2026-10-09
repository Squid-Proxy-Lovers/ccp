// Cephalopod Coordination Protocol
// Copyright (C) 2026 Squid Proxy Lovers
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

const JOURNAL_QUEUE_CAPACITY: usize = 4096;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JournalEntry {
    AuthTokenUsed {
        #[serde(alias = "token")]
        token_hash: String,
        last_used_at: String,
    },
    IssuedCert {
        session_id: i64,
        common_name: String,
        access_level: String,
        cert_pem: String,
        created_at: String,
        expires_at: String,
    },
    AddShelf {
        session_id: i64,
        shelf_name: String,
        description: String,
    },
    AddBook {
        session_id: i64,
        shelf_name: String,
        book_name: String,
        description: String,
    },
    AddEntry {
        session_id: i64,
        name: String,
        description: String,
        labels: Vec<String>,
        context: String,
        shelf_name: String,
        book_name: String,
        created_at: String,
        updated_at: String,
    },
    AppendEntry {
        session_id: i64,
        name: String,
        operation_id: String,
        client_common_name: String,
        agent_name: Option<String>,
        host_name: Option<String>,
        reason: Option<String>,
        appended_content: String,
        shelf_name: String,
        book_name: String,
        created_at: String,
    },
    TransferExported {
        session_id: i64,
        scope_json: String,
        bundle_sha256: String,
        entry_count: usize,
    },
    TransferImported {
        session_id: i64,
        scope_json: String,
        policy: String,
        bundle_sha256: String,
        entry_count: usize,
    },
    TransferImportFailed {
        session_id: i64,
        bundle_sha256: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalPosition {
    pub generation: String,
    pub byte_offset: u64,
}

#[derive(Serialize, Deserialize)]
struct JournalHeader {
    ccp_journal_generation: String,
}

enum JournalCommand {
    Append(Box<JournalEntry>),
    Flush(mpsc::SyncSender<anyhow::Result<JournalPosition>>),
    Checkpoint(mpsc::SyncSender<anyhow::Result<()>>),
    Shutdown(mpsc::SyncSender<anyhow::Result<()>>),
}

#[derive(Clone)]
pub struct JournalHandle {
    path: PathBuf,
    sender: mpsc::SyncSender<JournalCommand>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl JournalHandle {
    pub fn start(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create journal directory at {}", parent.display())
            })?;
        }
        repair_incomplete_tail(&path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open journal at {}", path.display()))?;
        let generation = if file.metadata()?.len() == 0 {
            write_header(&mut file)?
        } else {
            read_generation(&std::fs::read(&path)?)?
        };
        let (sender, receiver) = mpsc::sync_channel(JOURNAL_QUEUE_CAPACITY);
        let last_error = Arc::new(Mutex::new(None));
        thread::Builder::new()
            .name("ccp-journal-writer".to_string())
            .spawn({
                let last_error = Arc::clone(&last_error);
                let writer_path = path.clone();
                move || run_journal_writer(file, writer_path, generation, receiver, last_error)
            })
            .context("failed to spawn journal writer thread")?;
        Ok(Self {
            path,
            sender,
            last_error,
        })
    }

    pub fn append(&self, entry: JournalEntry) -> anyhow::Result<()> {
        self.check_health()?;
        self.sender
            .send(JournalCommand::Append(Box::new(entry)))
            .map_err(|_| self.send_error())?;
        self.check_health()
    }

    /// Drain and sync all queued entries, returning the exact recoverable boundary.
    pub fn flush(&self) -> anyhow::Result<JournalPosition> {
        self.check_health()?;
        let (tx, rx) = mpsc::sync_channel(1);
        self.sender
            .send(JournalCommand::Flush(tx))
            .map_err(|_| self.send_error())?;
        rx.recv()
            .context("journal writer dropped flush acknowledgement")?
    }

    /// Caller must have committed a snapshot and prevented concurrent mutations.
    /// The writer replaces the log atomically with a new generation so a crash
    /// cannot confuse an old snapshot byte offset with new journal records.
    pub fn checkpoint(&self) -> anyhow::Result<()> {
        self.check_health()?;
        let (tx, rx) = mpsc::sync_channel(1);
        self.sender
            .send(JournalCommand::Checkpoint(tx))
            .map_err(|_| self.send_error())?;
        rx.recv()
            .context("journal writer dropped checkpoint acknowledgement")?
    }

    pub fn shutdown(&self) -> anyhow::Result<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.sender
            .send(JournalCommand::Shutdown(tx))
            .map_err(|_| self.send_error())?;
        rx.recv()
            .context("journal writer dropped shutdown acknowledgement")??;
        self.check_health()?;
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) =
            Some("journal writer is not available".to_string());
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn check_health(&self) -> anyhow::Result<()> {
        match self
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            Some(message) => Err(anyhow!(message)),
            None => Ok(()),
        }
    }

    fn send_error(&self) -> anyhow::Error {
        self.last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .map(anyhow::Error::msg)
            .unwrap_or_else(|| anyhow!("journal writer is not available"))
    }
}

/// A crash may leave an incomplete final write. Only discard a malformed
/// unterminated final line; malformed newline-terminated records remain errors.
fn repair_incomplete_tail(path: &Path) -> anyhow::Result<()> {
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("failed to inspect journal tail"),
    };
    if contents.is_empty() || contents.ends_with(b"\n") {
        return Ok(());
    }
    let tail_start = contents
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let tail = &contents[tail_start..];
    let complete = serde_json::from_slice::<JournalEntry>(tail).is_ok()
        || (tail_start == 0 && serde_json::from_slice::<JournalHeader>(tail).is_ok());
    let mut file = OpenOptions::new().append(true).open(path)?;
    if complete {
        file.write_all(b"\n")?;
    } else {
        file.set_len(tail_start as u64)?;
        eprintln!("Discarded incomplete trailing journal record at byte {tail_start}");
    }
    file.sync_all()
        .context("failed to repair incomplete journal tail")
}

fn read_generation(contents: &[u8]) -> anyhow::Result<String> {
    let first = contents
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    match serde_json::from_slice::<JournalHeader>(first) {
        Ok(header) => Ok(header.ccp_journal_generation),
        // Headerless journals from earlier versions use the empty generation.
        Err(_) => Ok(String::new()),
    }
}

pub(crate) fn has_legacy_entries(path: &Path) -> anyhow::Result<bool> {
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("failed to inspect legacy journal"),
    };
    Ok(!contents.iter().all(u8::is_ascii_whitespace) && read_generation(&contents)?.is_empty())
}

pub fn load_entries(path: &Path) -> anyhow::Result<Vec<JournalEntry>> {
    load_entries_after(path, None)
}

pub fn load_entries_after(
    path: &Path,
    checkpoint: Option<&JournalPosition>,
) -> anyhow::Result<Vec<JournalEntry>> {
    let contents = match std::fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read journal at {}", path.display()));
        }
    };
    let generation = read_generation(&contents)?;
    let start = checkpoint
        .filter(|position| position.generation == generation)
        .map(|position| position.byte_offset)
        .unwrap_or(0);
    if start > contents.len() as u64 {
        return Err(anyhow!("journal is shorter than its persisted checkpoint"));
    }
    if start > 0 && contents[start as usize - 1] != b'\n' {
        return Err(anyhow!(
            "journal checkpoint is not at a complete record boundary"
        ));
    }
    let mut entries = Vec::new();
    let mut offset = 0u64;
    for line in contents.split_inclusive(|byte| *byte == b'\n') {
        let line_start = offset;
        offset += line.len() as u64;
        if line_start < start || line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if line_start == 0 && !generation.is_empty() {
            continue;
        }
        match serde_json::from_slice::<JournalEntry>(line) {
            Ok(entry) => entries.push(entry),
            Err(_) if !line.ends_with(b"\n") => {
                break;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to parse journal record at byte {line_start}")
                });
            }
        }
    }
    Ok(entries)
}

fn write_header(file: &mut File) -> anyhow::Result<String> {
    let generation = uuid::Uuid::new_v4().to_string();
    serde_json::to_writer(
        &mut *file,
        &JournalHeader {
            ccp_journal_generation: generation.clone(),
        },
    )?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(generation)
}

fn rotate_journal(file: &mut File, path: &Path, generation: &mut String) -> anyhow::Result<()> {
    let temporary = path.with_extension(format!("checkpoint-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut replacement = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&temporary)?;
        let new_generation = write_header(&mut replacement)?;
        std::fs::rename(&temporary, path).context("failed to replace journal at checkpoint")?;
        // Sync the containing directory to retain the replacement across power loss.
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        *file = replacement;
        *generation = new_generation;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn run_journal_writer(
    mut file: File,
    path: PathBuf,
    mut generation: String,
    receiver: mpsc::Receiver<JournalCommand>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    let mut pending = None;
    loop {
        let command = match pending.take().map(Ok).unwrap_or_else(|| receiver.recv()) {
            Ok(command) => command,
            Err(_) => break,
        };
        let result = match command {
            JournalCommand::Append(entry) => (|| {
                let mut buffer =
                    serde_json::to_vec(&entry).context("failed to serialize journal entry")?;
                buffer.push(b'\n');
                // Keep the original batched writes, but never consume an ordered
                // flush/checkpoint command before its preceding batch is written.
                for _ in 0..63 {
                    if buffer.len() >= 4 * 1024 * 1024 {
                        break;
                    }
                    match receiver.try_recv() {
                        Ok(JournalCommand::Append(entry)) => {
                            serde_json::to_writer(&mut buffer, &entry)
                                .context("failed to serialize journal entry")?;
                            buffer.push(b'\n');
                        }
                        Ok(command) => {
                            pending = Some(command);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                file.write_all(&buffer)
                    .context("failed to append journal batch")?;
                file.flush().context("failed to flush journal batch")
            })(),
            JournalCommand::Flush(done) => {
                let result = file
                    .sync_all()
                    .context("failed to sync journal")
                    .and_then(|_| {
                        Ok(JournalPosition {
                            generation: generation.clone(),
                            byte_offset: file.metadata()?.len(),
                        })
                    });
                let failure = result.as_ref().err().map(|error| error.to_string());
                if let Some(message) = &failure {
                    record_error(&last_error, anyhow!(message.clone()));
                }
                let _ = done.send(result);
                if failure.is_some() {
                    return;
                }
                Ok(())
            }
            JournalCommand::Checkpoint(done) => {
                let result = rotate_journal(&mut file, &path, &mut generation);
                let failure = result.as_ref().err().map(|error| error.to_string());
                if let Some(message) = &failure {
                    record_error(&last_error, anyhow!(message.clone()));
                }
                let _ = done.send(result);
                if failure.is_some() {
                    return;
                }
                Ok(())
            }
            JournalCommand::Shutdown(done) => {
                let result = file
                    .sync_all()
                    .context("failed to sync journal on shutdown");
                let _ = done.send(result);
                return;
            }
        };
        if let Err(error) = result {
            record_error(&last_error, error);
            return;
        }
    }
}

fn record_error(last_error: &Arc<Mutex<Option<String>>>, error: anyhow::Error) {
    *last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str) -> JournalEntry {
        JournalEntry::AddShelf {
            session_id: 1,
            shelf_name: name.to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn legacy_headerless_journal_recovers_and_honors_snapshot_boundary() {
        let path = std::env::temp_dir().join(format!("ccp-journal-{}", uuid::Uuid::new_v4()));
        let mut first = serde_json::to_vec(&sample("first")).unwrap();
        first.push(b'\n');
        std::fs::write(&path, &first).unwrap();
        let journal = JournalHandle::start(path.clone()).unwrap();
        let boundary = journal.flush().unwrap();
        assert!(boundary.generation.is_empty());
        journal.append(sample("second")).unwrap();
        journal.flush().unwrap();
        assert_eq!(load_entries_after(&path, Some(&boundary)).unwrap().len(), 1);
        journal.checkpoint().unwrap();
        journal.append(sample("third")).unwrap();
        journal.flush().unwrap();
        assert_eq!(load_entries_after(&path, Some(&boundary)).unwrap().len(), 1);
        journal.shutdown().unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn incomplete_final_write_is_repaired_before_new_appends() {
        let path = std::env::temp_dir().join(format!("ccp-journal-{}", uuid::Uuid::new_v4()));
        let mut first = serde_json::to_vec(&sample("first")).unwrap();
        first.extend_from_slice(b"\n{\"AddShelf\":");
        std::fs::write(&path, first).unwrap();
        let journal = JournalHandle::start(path.clone()).unwrap();
        journal.append(sample("second")).unwrap();
        journal.shutdown().unwrap();
        assert_eq!(load_entries(&path).unwrap().len(), 2);
        let bad_boundary = JournalPosition {
            generation: String::new(),
            byte_offset: 2,
        };
        assert!(load_entries_after(&path, Some(&bad_boundary)).is_err());
        std::fs::write(&path, b"malformed\n").unwrap();
        assert!(load_entries(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
}

#[cfg(test)]
mod ordering_tests {
    use super::*;

    #[test]
    fn flush_drains_bounded_batches_and_rotation_failure_closes_writes() {
        let directory =
            std::env::temp_dir().join(format!("ccp-journal-order-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("runtime-journal.jsonl");
        let journal = JournalHandle::start(path.clone()).unwrap();
        for index in 0..256 {
            journal
                .append(JournalEntry::AddShelf {
                    session_id: 1,
                    shelf_name: index.to_string(),
                    description: String::new(),
                })
                .unwrap();
        }
        let position = journal.flush().unwrap();
        assert_eq!(
            position.byte_offset,
            std::fs::metadata(&path).unwrap().len()
        );
        assert_eq!(load_entries(&path).unwrap().len(), 256);
        assert!(
            load_entries_after(&path, Some(&position))
                .unwrap()
                .is_empty()
        );
        // The checkpoint error must publish terminal health before its ACK.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(journal.checkpoint().is_err());
        assert!(
            journal
                .append(JournalEntry::AddShelf {
                    session_id: 1,
                    shelf_name: "after failure".into(),
                    description: String::new()
                })
                .is_err()
        );
        let _ = journal.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn complete_legacy_final_record_gets_separator_before_append() {
        let path =
            std::env::temp_dir().join(format!("ccp-journal-separator-{}", uuid::Uuid::new_v4()));
        let first = JournalEntry::AddShelf {
            session_id: 1,
            shelf_name: "first".into(),
            description: String::new(),
        };
        std::fs::write(&path, serde_json::to_vec(&first).unwrap()).unwrap();
        let journal = JournalHandle::start(path.clone()).unwrap();
        journal.append(first).unwrap();
        journal.shutdown().unwrap();
        assert_eq!(load_entries(&path).unwrap().len(), 2);
        std::fs::remove_file(path).unwrap();
    }
}
