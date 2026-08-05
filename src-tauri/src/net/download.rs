//! A multi-connection HTTP downloader with resume.
//!
//! One TCP connection rarely saturates a link when the server is far away, so a
//! transfer is split into ranges fetched in parallel and written straight into a
//! preallocated file. Progress for each range is journalled next to the download,
//! which is what makes a resume pick up where it stopped rather than starting over.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use crate::error::{Error, IoContext, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DownloadState {
    Queued,
    Probing,
    Running,
    Paused,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub id: String,
    pub url: String,
    pub destination: PathBuf,
    pub state: DownloadState,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: u64,
    pub bytes_per_second: u64,
    pub connections: usize,
    pub error: Option<String>,
}

/// What the journal remembers between runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    url: String,
    total_bytes: u64,
    /// Bytes already written for each range, in range order.
    completed: Vec<u64>,
    ranges: Vec<(u64, u64)>,
}

impl Journal {
    fn path_for(destination: &Path) -> PathBuf {
        destination.with_extension(format!(
            "{}.rtpart",
            destination
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default()
        ))
    }

    fn load(destination: &Path, url: &str, total: u64) -> Option<Journal> {
        let raw = std::fs::read(Journal::path_for(destination)).ok()?;
        let journal: Journal = serde_json::from_slice(&raw).ok()?;
        // A journal only applies to the same file from the same place.
        (journal.url == url && journal.total_bytes == total).then_some(journal)
    }

    fn save(&self, destination: &Path) {
        if let Ok(bytes) = serde_json::to_vec(self) {
            std::fs::write(Journal::path_for(destination), bytes).ok();
        }
    }

    fn discard(destination: &Path) {
        std::fs::remove_file(Journal::path_for(destination)).ok();
    }
}

/// Splits `total` into at most `parts` contiguous ranges, inclusive at both ends.
///
/// Ranges below `MIN_CHUNK` are pointless: the request overhead costs more than the
/// parallelism saves, so small files stay on a single connection.
pub fn plan_ranges(total: u64, parts: usize) -> Vec<(u64, u64)> {
    const MIN_CHUNK: u64 = 1024 * 1024;

    if total == 0 {
        return Vec::new();
    }
    let usable = parts.max(1).min(((total / MIN_CHUNK).max(1)) as usize);
    let chunk = total / usable as u64;

    (0..usable)
        .map(|index| {
            let start = chunk * index as u64;
            let end = if index == usable - 1 {
                total - 1
            } else {
                start + chunk - 1
            };
            (start, end)
        })
        .collect()
}

pub struct DownloadHandle {
    pub id: String,
    cancel: Arc<AtomicBool>,
    downloaded: Arc<AtomicU64>,
}

impl DownloadHandle {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn downloaded(&self) -> u64 {
        self.downloaded.load(Ordering::Relaxed)
    }
}

/// Downloads `url` into `destination`, reporting progress through `on_progress`.
pub async fn fetch<F>(
    client: reqwest::Client,
    id: String,
    url: String,
    destination: PathBuf,
    connections: usize,
    cancel: Arc<AtomicBool>,
    mut on_progress: F,
) -> Result<PathBuf>
where
    F: FnMut(DownloadProgress) + Send + 'static,
{
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).at(parent)?;
    }

    let report = |state: DownloadState,
                  total: Option<u64>,
                  done: u64,
                  speed: u64,
                  used: usize,
                  error: Option<String>| DownloadProgress {
        id: id.clone(),
        url: url.clone(),
        destination: destination.clone(),
        state,
        total_bytes: total,
        downloaded_bytes: done,
        bytes_per_second: speed,
        connections: used,
        error,
    };

    on_progress(report(DownloadState::Probing, None, 0, 0, 0, None));

    // A HEAD tells us the size and whether ranges are allowed. Servers that refuse
    // HEAD are common enough that a failure here falls back to a single stream.
    let head = client.head(&url).send().await.ok();
    let total = head
        .as_ref()
        .and_then(|r| r.content_length())
        .filter(|len| *len > 0);
    let accepts_ranges = head
        .as_ref()
        .and_then(|r| r.headers().get(reqwest::header::ACCEPT_RANGES).cloned())
        .map(|value| value.as_bytes() != b"none")
        .unwrap_or(false);

    let downloaded = Arc::new(AtomicU64::new(0));
    let started = Instant::now();

    let Some(total) = total.filter(|_| accepts_ranges && connections > 1) else {
        // Single stream: either the size is unknown or ranges are unavailable.
        return stream_whole(
            &client,
            &url,
            &destination,
            total,
            downloaded,
            cancel,
            started,
            report,
            on_progress,
        )
        .await;
    };

    let existing = Journal::load(&destination, &url, total);
    let ranges = existing
        .as_ref()
        .map(|j| j.ranges.clone())
        .unwrap_or_else(|| plan_ranges(total, connections));
    let mut completed = existing
        .as_ref()
        .map(|j| j.completed.clone())
        .unwrap_or_else(|| vec![0; ranges.len()]);
    if completed.len() != ranges.len() {
        completed = vec![0; ranges.len()];
    }

    downloaded.store(completed.iter().sum(), Ordering::Relaxed);

    // Preallocate so the ranges have somewhere to land.
    {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&destination)
            .at(&destination)?;
        file.set_len(total).at(&destination)?;
    }

    let progress: Vec<Arc<AtomicU64>> = completed
        .iter()
        .map(|done| Arc::new(AtomicU64::new(*done)))
        .collect();

    let mut tasks = Vec::new();
    for (index, (start, end)) in ranges.iter().copied().enumerate() {
        let already = progress[index].load(Ordering::Relaxed);
        if start + already > end {
            continue;
        }

        let client = client.clone();
        let url = url.clone();
        let destination = destination.clone();
        let cancel = Arc::clone(&cancel);
        let counter = Arc::clone(&downloaded);
        let slot = Arc::clone(&progress[index]);

        tasks.push(tokio::spawn(async move {
            fetch_range(
                client,
                url,
                destination,
                start + already,
                end,
                cancel,
                counter,
                slot,
            )
            .await
        }));
    }

    // Emit progress on a timer rather than per chunk, so the UI is not flooded.
    let ticker = {
        let downloaded = Arc::clone(&downloaded);
        let cancel = Arc::clone(&cancel);
        let progress = progress.clone();
        let destination = destination.clone();
        let url = url.clone();
        let ranges = ranges.clone();
        tokio::spawn(async move {
            let mut last = 0u64;
            let mut last_at = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(400)).await;
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let now = downloaded.load(Ordering::Relaxed);
                let elapsed = last_at.elapsed().as_secs_f64().max(0.001);
                let speed = ((now.saturating_sub(last)) as f64 / elapsed) as u64;
                last = now;
                last_at = Instant::now();

                Journal {
                    url: url.clone(),
                    total_bytes: total,
                    completed: progress.iter().map(|p| p.load(Ordering::Relaxed)).collect(),
                    ranges: ranges.clone(),
                }
                .save(&destination);

                if now >= total {
                    break;
                }
                let _ = speed;
            }
        })
    };

    let mut failure = None;
    for task in tasks {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failure = Some(error.to_string()),
            Err(error) => failure = Some(error.to_string()),
        }
    }
    ticker.abort();

    let done = downloaded.load(Ordering::Relaxed);
    let speed = (done as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;

    if cancel.load(Ordering::Relaxed) {
        on_progress(report(
            DownloadState::Paused,
            Some(total),
            done,
            speed,
            ranges.len(),
            None,
        ));
        return Err(Error::msg("download paused"));
    }

    if let Some(error) = failure {
        on_progress(report(
            DownloadState::Failed,
            Some(total),
            done,
            speed,
            ranges.len(),
            Some(error.clone()),
        ));
        return Err(Error::Network(error));
    }

    Journal::discard(&destination);
    on_progress(report(
        DownloadState::Complete,
        Some(total),
        total,
        speed,
        ranges.len(),
        None,
    ));
    Ok(destination)
}

#[allow(clippy::too_many_arguments)]
async fn fetch_range(
    client: reqwest::Client,
    url: String,
    destination: PathBuf,
    start: u64,
    end: u64,
    cancel: Arc<AtomicBool>,
    total_counter: Arc<AtomicU64>,
    range_counter: Arc<AtomicU64>,
) -> Result<()> {
    let response = client
        .get(&url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(Error::Network(format!(
            "server replied {} for range {start}-{end}",
            response.status()
        )));
    }

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&destination)
        .await
        .at(&destination)?;
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .at(&destination)?;

    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            file.flush().await.ok();
            return Ok(());
        }
        let chunk = chunk?;
        file.write_all(&chunk).await.at(&destination)?;
        total_counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        range_counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }

    file.flush().await.at(&destination)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn stream_whole<R, F>(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    total: Option<u64>,
    downloaded: Arc<AtomicU64>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    report: R,
    mut on_progress: F,
) -> Result<PathBuf>
where
    R: Fn(DownloadState, Option<u64>, u64, u64, usize, Option<String>) -> DownloadProgress,
    F: FnMut(DownloadProgress),
{
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        let message = format!("server replied {}", response.status());
        on_progress(report(
            DownloadState::Failed,
            total,
            0,
            0,
            1,
            Some(message.clone()),
        ));
        return Err(Error::Network(message));
    }

    let mut file = tokio::fs::File::create(destination).await.at(destination)?;
    let mut stream = response.bytes_stream();
    let mut last_emit = Instant::now();

    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::Relaxed) {
            file.flush().await.ok();
            return Err(Error::msg("download paused"));
        }
        let chunk = chunk?;
        file.write_all(&chunk).await.at(destination)?;
        let done = downloaded.fetch_add(chunk.len() as u64, Ordering::Relaxed) + chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(400) {
            let speed = (done as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
            on_progress(report(DownloadState::Running, total, done, speed, 1, None));
            last_emit = Instant::now();
        }
    }

    file.flush().await.at(destination)?;
    let done = downloaded.load(Ordering::Relaxed);
    let speed = (done as f64 / started.elapsed().as_secs_f64().max(0.001)) as u64;
    on_progress(report(
        DownloadState::Complete,
        Some(done),
        done,
        speed,
        1,
        None,
    ));
    Ok(destination.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_cover_the_file_exactly_once() {
        let total = 100 * 1024 * 1024;
        let ranges = plan_ranges(total, 8);
        assert_eq!(ranges.len(), 8);
        assert_eq!(ranges[0].0, 0);
        assert_eq!(ranges.last().unwrap().1, total - 1);

        // Contiguous, no gaps and no overlap.
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].1 + 1, pair[1].0);
        }
        let covered: u64 = ranges.iter().map(|(s, e)| e - s + 1).sum();
        assert_eq!(covered, total);
    }

    #[test]
    fn small_files_stay_on_one_connection() {
        // Below the one-megabyte floor, extra connections only add round trips.
        assert_eq!(plan_ranges(500_000, 8).len(), 1);
        assert_eq!(plan_ranges(500_000, 8)[0], (0, 499_999));
    }

    #[test]
    fn connection_count_is_capped_by_size() {
        // A three-megabyte file cannot usefully use sixteen connections.
        assert_eq!(plan_ranges(3 * 1024 * 1024, 16).len(), 3);
    }

    #[test]
    fn an_empty_body_produces_no_ranges() {
        assert!(plan_ranges(0, 8).is_empty());
    }

    #[test]
    fn a_single_connection_is_one_range() {
        let ranges = plan_ranges(50 * 1024 * 1024, 1);
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0], (0, 50 * 1024 * 1024 - 1));
    }

    #[test]
    fn the_journal_is_only_reused_for_the_same_source() {
        let dir = std::env::temp_dir().join("roundtable-journal");
        std::fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("file.bin");

        let journal = Journal {
            url: "https://example.com/a".into(),
            total_bytes: 1000,
            completed: vec![10, 20],
            ranges: vec![(0, 499), (500, 999)],
        };
        journal.save(&destination);

        assert!(Journal::load(&destination, "https://example.com/a", 1000).is_some());
        // A different URL or a changed size means the partial file is not ours.
        assert!(Journal::load(&destination, "https://example.com/b", 1000).is_none());
        assert!(Journal::load(&destination, "https://example.com/a", 2000).is_none());

        Journal::discard(&destination);
        assert!(Journal::load(&destination, "https://example.com/a", 1000).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
