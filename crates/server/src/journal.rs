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

enum JournalCommand {
    Append(JournalEntry),
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

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open journal at {}", path.display()))?;
        let (sender, receiver) = mpsc::sync_channel(JOURNAL_QUEUE_CAPACITY);
        let last_error = Arc::new(Mutex::new(None));

        thread::Builder::new()
            .name("ccp-journal-writer".to_string())
            .spawn({
                let last_error = Arc::clone(&last_error);
                move || run_journal_writer(file, receiver, last_error)
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
            .send(JournalCommand::Append(entry))
            .map_err(|_| self.send_error())?;
        self.check_health()?;
        Ok(())
    }

    pub fn shutdown(&self) -> anyhow::Result<()> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        self.sender
            .send(JournalCommand::Shutdown(ack_tx))
            .map_err(|_| self.send_error())?;
        ack_rx
            .recv()
            .context("journal writer dropped shutdown acknowledgement")??;
        self.check_health()?;
        *self.last_error.lock().unwrap_or_else(|e| e.into_inner()) =
            Some("journal writer is not available".to_string());
        Ok(())
    }

    pub fn truncate_blocking(&self) -> anyhow::Result<()> {
        std::fs::write(&self.path, [])
            .with_context(|| format!("failed to truncate journal at {}", self.path.display()))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn check_health(&self) -> anyhow::Result<()> {
        let error = self
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match error {
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

pub fn load_entries(path: &Path) -> anyhow::Result<Vec<JournalEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read journal at {}", path.display()))?;
    let mut entries = Vec::new();
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        entries.push(
            serde_json::from_str::<JournalEntry>(line)
                .with_context(|| format!("failed to parse journal line: {line}"))?,
        );
    }
    Ok(entries)
}

fn run_journal_writer(
    mut file: File,
    receiver: mpsc::Receiver<JournalCommand>,
    last_error: Arc<Mutex<Option<String>>>,
) {
    while let Ok(command) = receiver.recv() {
        match command {
            JournalCommand::Append(entry) => {
                // Batch: serialize this entry plus any pending entries, then flush once
                let mut buf = match serde_json::to_vec(&entry) {
                    Ok(mut line) => {
                        line.push(b'\n');
                        line
                    }
                    Err(error) => {
                        record_error(
                            &last_error,
                            anyhow::Error::new(error).context("failed to serialize journal entry"),
                        );
                        return;
                    }
                };
                // Drain all pending commands without blocking
                while let Ok(next) = receiver.try_recv() {
                    match next {
                        JournalCommand::Append(next_entry) => {
                            match serde_json::to_vec(&next_entry) {
                                Ok(mut line) => {
                                    line.push(b'\n');
                                    buf.extend_from_slice(&line);
                                }
                                Err(error) => {
                                    record_error(
                                        &last_error,
                                        anyhow::Error::new(error)
                                            .context("failed to serialize journal entry"),
                                    );
                                    return;
                                }
                            }
                        }
                        JournalCommand::Shutdown(done) => {
                            // Write pending batch before shutdown
                            if let Err(error) = file
                                .write_all(&buf)
                                .and_then(|_| file.flush())
                                .context("failed to flush journal on shutdown")
                            {
                                let _ = done.send(Err(error));
                            } else {
                                let _ = done.send(Ok(()));
                            }
                            return;
                        }
                    }
                }
                if let Err(error) = file
                    .write_all(&buf)
                    .context("failed to append journal batch")
                    .and_then(|_| file.flush().context("failed to flush journal batch"))
                {
                    record_error(&last_error, error);
                    return;
                }
            }
            JournalCommand::Shutdown(done) => {
                let result = file.flush().context("failed to flush journal on shutdown");
                let _ = done.send(result);
                return;
            }
        }
    }
}

fn record_error(last_error: &Arc<Mutex<Option<String>>>, error: anyhow::Error) {
    let message = error.to_string();
    *last_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(message);
}
