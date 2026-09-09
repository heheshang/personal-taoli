//! 归档写入器。

use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread,
};

use anyhow::{Context, Result, bail};

use crate::market::unix_timestamp_ms;

use super::{
    SCHEMA_VERSION,
    event_types::{ArchiveGap, ArchiveRecord, PendingGap},
};

static RUN_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub struct ArchiveWriter {
    run_id: String,
    raw_retention_days: u16,
    next_ordinal: u64,
    sender: Option<mpsc::SyncSender<ArchiveRecord>>,
    pending_overflow: Arc<Mutex<Option<PendingGap>>>,
    worker: Option<thread::JoinHandle<Result<()>>>,
}

impl ArchiveWriter {
    pub fn start(
        path: impl AsRef<Path>,
        gap_path: impl AsRef<Path>,
        queue_capacity: usize,
        raw_retention_days: u16,
    ) -> Result<Self> {
        if queue_capacity == 0 {
            bail!("archive queue capacity must be positive");
        }
        let started_at_ms = unix_timestamp_ms()?;
        let run_id = format!(
            "{started_at_ms}-{}-{}",
            std::process::id(),
            RUN_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        );
        let (sender, receiver) = mpsc::sync_channel(queue_capacity);
        let pending_overflow = Arc::new(Mutex::new(None));
        let worker_pending = Arc::clone(&pending_overflow);
        let worker_run_id = run_id.clone();
        let archive_path = path.as_ref().to_owned();
        let gaps_path = gap_path.as_ref().to_owned();
        let mut startup_gaps = Vec::new();
        for (candidate, label) in [
            (&archive_path, "archive"),
            (&gaps_path, "archive gap journal"),
        ] {
            let discarded = repair_incomplete_tail(candidate)?;
            if discarded > 0 {
                let event_id = format!("{run_id}:startup-{label}-tail-repair");
                startup_gaps.push(ArchiveGap {
                    schema_version: SCHEMA_VERSION,
                    run_id: run_id.clone(),
                    first_event_id: event_id.clone(),
                    last_event_id: event_id,
                    first_observed_at_ms: started_at_ms,
                    last_observed_at_ms: started_at_ms,
                    dropped_events: 0,
                    reason: format!("discarded {discarded} incomplete bytes from {label}"),
                    recorded_at_ms: started_at_ms,
                });
            }
        }
        let worker = thread::Builder::new()
            .name("market-archive".into())
            .spawn(move || {
                writer_loop(
                    receiver,
                    &archive_path,
                    &gaps_path,
                    &worker_run_id,
                    worker_pending,
                    startup_gaps,
                )
            })
            .context("failed to spawn market archive writer")?;
        Ok(Self {
            run_id,
            raw_retention_days,
            next_ordinal: 1,
            sender: Some(sender),
            pending_overflow,
            worker: Some(worker),
        })
    }

    pub fn submit(&mut self, event: super::event_types::ArchivedEvent) -> Result<()> {
        let record = ArchiveRecord::new(
            &self.run_id,
            self.next_ordinal,
            self.raw_retention_days,
            event,
        )?;
        self.next_ordinal += 1;
        let sender = self
            .sender
            .as_ref()
            .context("archive writer is already closed")?;
        match sender.try_send(record) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(record)) => {
                let mut pending = self
                    .pending_overflow
                    .lock()
                    .map_err(|_| anyhow::anyhow!("archive gap state is poisoned"))?;
                match pending.as_mut() {
                    Some(gap) => gap.extend(&record),
                    None => *pending = Some(PendingGap::from_record(&record)),
                }
                Ok(())
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                bail!("archive writer stopped unexpectedly")
            }
        }
    }

    pub fn finish(mut self) -> Result<()> {
        self.close()
    }

    fn close(&mut self) -> Result<()> {
        self.sender.take();
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("archive writer thread panicked"))?
    }
}

impl Drop for ArchiveWriter {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("archive shutdown failed: {error:#}");
        }
    }
}

fn writer_loop(
    receiver: mpsc::Receiver<ArchiveRecord>,
    archive_path: &Path,
    gap_path: &Path,
    run_id: &str,
    pending_overflow: Arc<Mutex<Option<PendingGap>>>,
    startup_gaps: Vec<ArchiveGap>,
) -> Result<()> {
    let mut archive = open_append(archive_path).ok();
    let mut gaps = open_append(gap_path).ok();
    for gap in startup_gaps {
        write_gap(&mut gaps, gap_path, gap);
    }
    while let Ok(record) = receiver.recv() {
        flush_overflow_gap(&mut gaps, gap_path, run_id, &pending_overflow);
        let write_result = match archive.as_mut() {
            Some(writer) => write_json_line(writer, &record),
            None => Err(anyhow::anyhow!("archive file is unavailable")),
        };
        if let Err(error) = write_result {
            write_gap(
                &mut gaps,
                gap_path,
                PendingGap::from_record(&record)
                    .finish(run_id, &format!("archive write failed: {error:#}")),
            );
            archive = open_append(archive_path).ok();
        }
    }
    flush_overflow_gap(&mut gaps, gap_path, run_id, &pending_overflow);
    if let Some(writer) = archive.as_mut() {
        writer.flush().context("failed to flush archive")?;
    }
    if let Some(writer) = gaps.as_mut() {
        writer
            .flush()
            .context("failed to flush archive gap journal")?;
    }
    Ok(())
}

fn flush_overflow_gap(
    gaps: &mut Option<BufWriter<File>>,
    gap_path: &Path,
    run_id: &str,
    pending: &Mutex<Option<PendingGap>>,
) {
    let gap = pending.lock().ok().and_then(|mut gap| gap.take());
    if let Some(gap) = gap {
        write_gap(gaps, gap_path, gap.finish(run_id, "archive queue overflow"));
    }
}

fn write_gap(gaps: &mut Option<BufWriter<File>>, gap_path: &Path, gap: ArchiveGap) {
    if gaps.is_none() {
        *gaps = open_append(gap_path).ok();
    }
    let result = match gaps.as_mut() {
        Some(writer) => write_json_line(writer, &gap),
        None => Err(anyhow::anyhow!("gap journal is unavailable")),
    };
    if let Err(error) = result {
        *gaps = None;
        eprintln!(
            "ARCHIVE_GAP {}..{} count={} reason={} journal_error={error:#}",
            gap.first_event_id, gap.last_event_id, gap.dropped_events, gap.reason,
        );
    }
}

pub(crate) fn repair_incomplete_tail(path: &Path) -> Result<u64> {
    use std::io::{Read, Seek, SeekFrom};

    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect archive tail {}", path.display()));
        }
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return Ok(0);
    }

    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("failed to open archive tail {}", path.display()))?;
    let original_len = metadata.len();
    file.seek(SeekFrom::End(-1))?;
    let mut final_byte = [0_u8; 1];
    file.read_exact(&mut final_byte)?;
    if final_byte[0] == b'\n' {
        return Ok(0);
    }

    const CHUNK_SIZE: usize = 8 * 1024;
    let mut buffer = [0_u8; CHUNK_SIZE];
    let mut cursor = original_len;
    let complete_len = loop {
        let start = cursor.saturating_sub(CHUNK_SIZE as u64);
        let len = usize::try_from(cursor - start).expect("tail chunk length fits usize");
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buffer[..len])?;
        if let Some(index) = buffer[..len].iter().rposition(|byte| *byte == b'\n') {
            break start + index as u64 + 1;
        }
        if start == 0 {
            break 0;
        }
        cursor = start;
    };
    file.set_len(complete_len)?;
    file.sync_data()
        .with_context(|| format!("failed to sync repaired archive tail {}", path.display()))?;
    Ok(original_len - complete_len)
}

fn open_append(path: &Path) -> Result<BufWriter<File>> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create archive directory {}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open archive {}", path.display()))?;
    Ok(BufWriter::new(file))
}

fn write_json_line<T: serde::Serialize>(writer: &mut BufWriter<File>, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}
