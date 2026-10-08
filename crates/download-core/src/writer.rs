//! Dedicated disk-writer thread for one download.
//!
//! Connections never touch the file. They claim byte ranges in the segment
//! table and send the bytes here through a bounded channel (backpressure).
//! The writer performs positional writes and then advances the segment's
//! `written` cursor.
//!
//! A checkpoint takes a snapshot of the table, then fsyncs the file, and
//! only then hands the snapshot to the caller for persisting. Every byte
//! recorded as written in a persisted snapshot is therefore durable, which
//! makes resume after a crash or power loss safe.

use std::fs::File;
use std::sync::Arc;
use std::thread::JoinHandle;

use bytes::Bytes;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use velox_segments::{PersistedSegment, SegmentTable};

use crate::error::{EngineError, EngineResult};
use crate::fsutil;

pub(crate) enum WriteCmd {
    Data {
        index: usize,
        offset: u64,
        data: Bytes,
    },
    Checkpoint(oneshot::Sender<EngineResult<Vec<PersistedSegment>>>),
    Finish(oneshot::Sender<EngineResult<()>>),
}

/// Handle to the writer thread.
pub(crate) struct DiskWriter {
    tx: mpsc::Sender<WriteCmd>,
    error: Arc<Mutex<Option<EngineError>>>,
    thread: Option<JoinHandle<()>>,
}

/// Chunks buffered between the network and the disk.
const QUEUE_DEPTH: usize = 256;

impl DiskWriter {
    pub fn spawn(file: File, table: Arc<Mutex<SegmentTable>>, name: String) -> Self {
        let (tx, mut rx) = mpsc::channel::<WriteCmd>(QUEUE_DEPTH);
        let error: Arc<Mutex<Option<EngineError>>> = Arc::default();
        let err2 = error.clone();
        let thread = std::thread::Builder::new()
            .name(format!("velox-writer-{name}"))
            .spawn(move || {
                let mut failed = false;
                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        WriteCmd::Data {
                            index,
                            offset,
                            data,
                        } => {
                            if failed {
                                continue;
                            }
                            if let Err(e) = fsutil::write_all_at(&file, &data, offset) {
                                tracing::error!(error = %e, "disk write failed");
                                *err2.lock() = Some(EngineError::from_io(e));
                                failed = true;
                                continue;
                            }
                            if let Err(e) =
                                table.lock().mark_written(index, offset, data.len() as u64)
                            {
                                *err2.lock() = Some(EngineError::Other(e.to_string()));
                                failed = true;
                            }
                        }
                        WriteCmd::Checkpoint(reply) => {
                            if failed {
                                let _ = reply.send(Err(err2
                                    .lock()
                                    .clone()
                                    .unwrap_or(EngineError::Io("write failed".into()))));
                                continue;
                            }
                            let snapshot = table.lock().snapshot();
                            let res = file
                                .sync_data()
                                .map(|_| snapshot)
                                .map_err(EngineError::from_io);
                            let _ = reply.send(res);
                        }
                        WriteCmd::Finish(reply) => {
                            let res = if failed {
                                Err(err2
                                    .lock()
                                    .clone()
                                    .unwrap_or(EngineError::Io("write failed".into())))
                            } else {
                                file.sync_all().map_err(EngineError::from_io)
                            };
                            let _ = reply.send(res);
                            break;
                        }
                    }
                }
            })
            .expect("spawn writer thread");
        Self {
            tx,
            error,
            thread: Some(thread),
        }
    }

    /// Sender for connections.
    pub fn sender(&self) -> mpsc::Sender<WriteCmd> {
        self.tx.clone()
    }

    /// The first write error, if any.
    pub fn error(&self) -> Option<EngineError> {
        self.error.lock().clone()
    }

    /// Flush queued writes, fsync, and return a durable snapshot.
    pub async fn checkpoint(&self) -> EngineResult<Vec<PersistedSegment>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Checkpoint(tx))
            .await
            .map_err(|_| EngineError::Io("writer stopped".into()))?;
        rx.await
            .map_err(|_| EngineError::Io("writer stopped".into()))?
    }

    /// Flush, fsync and close the file.
    pub async fn finish(mut self) -> EngineResult<()> {
        let (tx, rx) = oneshot::channel();
        let sent = self.tx.send(WriteCmd::Finish(tx)).await;
        let res = match sent {
            Ok(()) => rx
                .await
                .map_err(|_| EngineError::Io("writer stopped".into()))?,
            Err(_) => Err(EngineError::Io("writer stopped".into())),
        };
        if let Some(t) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || t.join()).await;
        }
        res
    }
}

impl Drop for DiskWriter {
    fn drop(&mut self) {
        // If `finish` was not called, the thread exits once every sender
        // (ours and the connections') is dropped.
        if let Some(t) = self.thread.take() {
            std::mem::drop(t);
        }
    }
}
