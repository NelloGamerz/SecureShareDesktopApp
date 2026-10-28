//! What one transfer actually did, written down when it ends.
//!
//! Every transfer ends once — completed, failed or cancelled — and until now
//! that ending left nothing behind but a status. This module records the rest:
//! how long each phase took, what the link actually sustained, where the time
//! went, and what the process cost while it was doing it.
//!
//! # Two reports per transfer, not one
//!
//! The sender and the receiver are different processes, often on different
//! machines. Each writes its own report, into its own application-data
//! directory, and both carry the transfer id and a `role` so they can be put
//! beside each other. Nothing is sent across the wire for reporting: the frozen
//! v1 protocol is untouched.
//!
//! # Measured, estimated, or absent
//!
//! A metric is only written when something observed it. Anything this side
//! cannot see — the receiver's assembly time on a sender report, the process
//! CPU when the sampler got no sample, first-chunk timings for a transfer that
//! never sent a chunk — is `null` and named in `unavailable`. A report that
//! guesses is worse than one that is incomplete.
//!
//! # Bounded
//!
//! Latencies go into a fixed log-scale histogram, not a list: 40 buckets cover
//! a microsecond to ten minutes, so a million chunks cost the same memory as
//! ten. Everything else is a counter or a timestamp.

use crate::models::transfer::TransferStatus;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, ProcessesToUpdate, System};

/// The report format this build writes.
pub const REPORT_SCHEMA_VERSION: u32 = 2;

/// How often the resource sampler reads the process.
const SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// Log-scale latency buckets: `2^b` nanoseconds, from 1 ns to about 10 minutes.
const LATENCY_BUCKETS: usize = 40;

/// `first_*_ns` before anything has happened.
const UNSET: u64 = u64::MAX;

/// Which side of the transfer wrote a report.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Sender,
    Receiver,
}

/// Latency percentiles, from a bounded histogram.
#[derive(Debug, Clone, Serialize)]
pub struct LatencySummary {
    pub count: u64,
    /// Milliseconds. `null` when nothing was recorded.
    pub mean_ms: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_ms: Option<f64>,
    /// The percentiles are bucket upper bounds exact to within a factor of two.
    pub resolution: &'static str,
}

/// A fixed-size log-scale histogram of durations.
pub struct LatencyHistogram {
    buckets: [u32; LATENCY_BUCKETS],
    count: u64,
    sum_ns: u64,
    max_ns: u64,
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self {
            buckets: [0; LATENCY_BUCKETS],
            count: 0,
            sum_ns: 0,
            max_ns: 0,
        }
    }
}

impl LatencyHistogram {
    fn from_persisted(persisted: &PersistedLatency) -> Self {
        let mut histogram = Self::default();

        for (index, count) in persisted.buckets.iter().take(LATENCY_BUCKETS).enumerate() {
            histogram.buckets[index] = *count;
        }

        histogram.count = persisted.count;
        histogram.sum_ns = persisted.sum_ns;
        histogram.max_ns = persisted.max_ns;

        histogram
    }

    pub fn record(&mut self, nanos: u64) {
        self.count += 1;
        self.sum_ns = self.sum_ns.saturating_add(nanos);
        self.max_ns = self.max_ns.max(nanos);

        let bucket = bucket_of(nanos);

        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
    }

    pub fn summary(&self) -> LatencySummary {
        if self.count == 0 {
            return LatencySummary {
                count: 0,
                mean_ms: None,
                p50_ms: None,
                p95_ms: None,
                p99_ms: None,
                max_ms: None,
                resolution: "log2 nanosecond buckets (upper bound, within a factor of 2)",
            };
        }

        /*
         * A bucket's upper bound is the next power of two, which can be above
         * the largest sample that landed in it. Clamping each percentile to the
         * observed maximum keeps the summary inside the data: a p50 above the
         * max is not a rounding artefact, it is a lie. The percentiles are also
         * forced non-decreasing, so p50 <= p95 <= p99 <= max holds however the
         * buckets fell.
         */
        let p50 = self.percentile(50.0).min(self.max_ns);
        let p95 = self.percentile(95.0).min(self.max_ns).max(p50);
        let p99 = self.percentile(99.0).min(self.max_ns).max(p95);

        LatencySummary {
            count: self.count,
            mean_ms: Some(nanos_to_ms(self.sum_ns as f64 / self.count as f64)),
            p50_ms: Some(nanos_to_ms(p50 as f64)),
            p95_ms: Some(nanos_to_ms(p95 as f64)),
            p99_ms: Some(nanos_to_ms(p99 as f64)),
            max_ms: Some(nanos_to_ms(self.max_ns as f64)),
            resolution: "log2 nanosecond buckets, clamped to the observed maximum",
        }
    }

    /// The upper bound of the bucket `percent` of the samples fall at or below.
    ///
    /// Never above the largest recorded sample; the caller clamps anyway.
    fn percentile(&self, percent: f64) -> u64 {
        let wanted = ((self.count as f64) * percent / 100.0).ceil() as u64;
        let mut seen = 0u64;

        for (bucket, count) in self.buckets.iter().enumerate() {
            seen += *count as u64;

            if seen >= wanted.max(1) {
                return bucket_upper_ns(bucket).min(self.max_ns);
            }
        }

        self.max_ns
    }
}

fn bucket_of(nanos: u64) -> usize {
    if nanos <= 1 {
        return 0;
    }

    let highest_bit = 63 - nanos.leading_zeros() as usize;

    highest_bit.min(LATENCY_BUCKETS - 1)
}

fn bucket_upper_ns(bucket: usize) -> u64 {
    if bucket == 0 {
        return 1;
    }

    (1u64 << bucket.min(63)).saturating_mul(2)
}

fn nanos_to_ms(nanos: f64) -> f64 {
    (nanos / 1_000_000.0 * 1000.0).round() / 1000.0
}

/// Process CPU and memory, sampled while the transfer runs.
#[derive(Default)]
pub struct ResourceSampler {
    peak_memory_bytes: AtomicU64,
    cpu_sum_milli_percent: AtomicU64,
    cpu_peak_milli_percent: AtomicU64,
    samples: AtomicU64,
    stop: Arc<AtomicBool>,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl ResourceSampler {
    /// Begin sampling this process.
    ///
    /// Each transfer samples the whole process, so a report for one of several
    /// concurrent transfers sees the others' CPU in its own numbers. That is a
    /// property of the operating system, not of this code, and is recorded in
    /// every report's notes.
    pub fn start() -> Arc<Self> {
        let sampler = Arc::new(Self::default());

        /*
         * No runtime, no sampler: a telemetry built outside tokio — a unit test
         * of the transfer state, for instance — reports its resources as
         * unavailable rather than panicking.
         */
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return sampler;
        };

        let inner = Arc::clone(&sampler);
        let stop = Arc::clone(&sampler.stop);

        let task = handle.spawn(async move {
            let mut system = System::new();
            let pid = Pid::from_u32(std::process::id());

            // The first refresh establishes the baseline `cpu_usage` needs.
            system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

            loop {
                if stop.load(Ordering::Relaxed) {
                    break;
                }

                tokio::time::sleep(SAMPLE_INTERVAL).await;

                if stop.load(Ordering::Relaxed) {
                    break;
                }

                system.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);

                let Some(process) = system.process(pid) else {
                    continue;
                };

                let memory = process.memory();
                let cpu = process.cpu_usage();

                inner.peak_memory_bytes.fetch_max(memory, Ordering::Relaxed);

                let milli_percent = (cpu * 1000.0).max(0.0) as u64;

                inner
                    .cpu_sum_milli_percent
                    .fetch_add(milli_percent, Ordering::Relaxed);
                inner
                    .cpu_peak_milli_percent
                    .fetch_max(milli_percent, Ordering::Relaxed);
                inner.samples.fetch_add(1, Ordering::Relaxed);
            }
        });

        *sampler.task.lock().expect("sampler lock poisoned") = Some(task);

        sampler
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn summary(&self) -> ResourceSummary {
        let samples = self.samples.load(Ordering::Relaxed);

        if samples == 0 {
            return ResourceSummary {
                samples: 0,
                cpu_average_percent: None,
                cpu_peak_percent: None,
                memory_peak_bytes: None,
                logical_cores: logical_cores(),
                note: "no resource sample was taken before the transfer ended",
            };
        }

        let sum = self.cpu_sum_milli_percent.load(Ordering::Relaxed);

        ResourceSummary {
            samples,
            cpu_average_percent: Some(
                (sum as f64 / samples as f64 / 1000.0 * 100.0).round() / 100.0,
            ),
            cpu_peak_percent: Some(
                (self.cpu_peak_milli_percent.load(Ordering::Relaxed) as f64 / 1000.0 * 100.0)
                    .round()
                    / 100.0,
            ),
            memory_peak_bytes: Some(self.peak_memory_bytes.load(Ordering::Relaxed)),
            logical_cores: logical_cores(),
            note: "percent of one core (sysinfo sums every thread), so 100% is one busy core                    and 400% is four; process-wide, so concurrent transfers share these numbers",
        }
    }
}

fn logical_cores() -> u32 {
    std::thread::available_parallelism()
        .map(|cores| cores.get() as u32)
        .unwrap_or(1)
}

impl Drop for ResourceSampler {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ResourceSummary {
    pub samples: u64,
    /// Percent of one core, averaged over the samples. Above 100 means several
    /// cores were busy at once.
    pub cpu_average_percent: Option<f64>,
    /// Percent of one core at the busiest sample; `peak / 100` is roughly how
    /// many cores were busy then.
    pub cpu_peak_percent: Option<f64>,
    pub memory_peak_bytes: Option<u64>,
    /// How many cores the machine has, so the percentages can be read.
    pub logical_cores: u32,
    pub note: &'static str,
}

/// One file's share of the transfer.
#[derive(Debug, Clone, Serialize)]
pub struct FileReport {
    pub file_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_path: Option<String>,
    pub size_bytes: u64,
    pub chunks: u64,
    pub chunks_done: u64,
    pub bytes_done: u64,
    pub retries: u64,
    pub failed_chunks: u64,
    /// Seconds from this file's first chunk to its last acknowledgement.
    pub duration_seconds: Option<f64>,
    pub throughput_mib_s: Option<f64>,
    pub outcome: &'static str,
}

/// One measured part of a chunk's life, as a share of the whole.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseShare {
    pub total_seconds: Option<f64>,
    pub share_percent: Option<f64>,
    pub latency: LatencySummary,
}

/// Where a chunk's time went.
///
/// This is the evidence for "the bottleneck is X": the parts are measured
/// separately on every attempt, and `dominant` is whichever of them holds the
/// most of the measured time. Latency minus their sum is retry backoff, lock
/// waits and scheduling — visible, and not attributed to any of the parts.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkBreakdown {
    pub read: PhaseShare,
    /// zstd, on the blocking pool. Absent when the receiver cannot decode it.
    pub compress: PhaseShare,
    /// Sealing, wall clock (cipher plus the wait for a blocking thread).
    pub encrypt: PhaseShare,
    /// The part of `encrypt` that was the cipher itself.
    pub encrypt_cpu: PhaseShare,
    pub http: PhaseShare,
    pub dominant: Option<String>,
}

/// What the caller knows that the telemetry could not observe.
pub struct Finish {
    pub transfer_id: String,
    pub status: TransferStatus,
    pub total_bytes: u64,
    pub completed_bytes: u64,
    pub total_chunks: u64,
    pub completed_chunks: u64,
    pub files: Vec<FileDescriptor>,
    pub retries: u64,
    pub failure_stage: Option<String>,
    pub failure_reason: Option<String>,
    pub cancellation_reason: Option<String>,
    /// Extra phase timings the platform measured (receiver side).
    pub phases: PhaseTimings,
    pub unavailable: Vec<String>,
    /// True when this report was rebuilt from what a restart found, rather than
    /// written as the transfer ended.
    pub recovered: bool,
}

/// A file the transfer was planned around.
#[derive(Debug, Clone)]
pub struct FileDescriptor {
    pub file_id: String,
    pub relative_path: Option<String>,
    pub size: u64,
    pub expected_chunks: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PhaseTimings {
    pub initialization_seconds: Option<f64>,
    pub handshake_seconds: Option<f64>,
    pub upload_seconds: Option<f64>,
    pub chunk_persistence_seconds: Option<f64>,
    pub final_assembly_seconds: Option<f64>,
    pub verification_seconds: Option<f64>,
    pub publication_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Throughput {
    pub bytes_acknowledged: u64,
    /// The window the throughput is measured over, named so it cannot be
    /// mistaken for something else.
    pub denominator: &'static str,
    pub window_seconds: Option<f64>,
    pub mib_s: Option<f64>,
    pub gib_s: Option<f64>,
    /// Chunk body bytes actually sent, after compression. Excludes HTTP
    /// headers and framing; `null` fields below are null when nothing was sent.
    pub wire_bytes: u64,
    /// Wire throughput over the same window as [`Self::mib_s`]. This is the
    /// number to compare against the link; `mib_s` counts plaintext.
    pub wire_mib_s: Option<f64>,
    /// Plaintext acknowledged per wire byte. 1.0 means compression did nothing.
    pub compression_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferReport {
    pub schema_version: u32,
    pub role: Role,
    pub transfer_id: String,
    pub status: String,
    pub started_at_unix_ms: Option<u64>,
    pub finished_at_unix_ms: u64,
    /// `null` when no duration was measured — never `0.000` standing in for
    /// "unknown".
    pub duration_seconds: Option<f64>,
    pub total_files: u64,
    /// Published by the receiver, or acknowledged and confirmed by it.
    pub completed_files: u64,
    /// Every chunk acknowledged, publication not confirmed on this side.
    pub unknown_files: u64,
    pub remaining_files: u64,
    pub total_bytes: u64,
    pub completed_bytes: u64,
    pub remaining_bytes: u64,
    pub total_chunks: u64,
    pub completed_chunks: u64,
    pub throughput: Throughput,
    pub first_chunk_start_seconds: Option<f64>,
    pub first_chunk_ack_seconds: Option<f64>,
    pub chunk_latency: LatencySummary,
    /// Which measured part of a chunk held the most time, and the shares.
    pub chunk_breakdown: ChunkBreakdown,
    pub resources: ResourceSummary,
    pub phases: PhaseTimings,
    pub retries: u64,
    pub failed_chunks: u64,
    pub failure_stage: Option<String>,
    pub failure_reason: Option<String>,
    pub cancellation_reason: Option<String>,
    pub files: Vec<FileReport>,
    pub compression_enabled: bool,
    pub integrity_digests_enabled: bool,
    pub unavailable: Vec<String>,
    /// True when rebuilt from persisted telemetry after a restart.
    pub recovered: bool,
    pub notes: Vec<String>,
}

/// The live counters, as a checkpoint writes them.
///
/// Every field is a measurement that already happened. A report recovered after
/// a restart is built from this, so it describes the run that took place
/// instead of the instant the process came back.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersistedTelemetry {
    pub started_unix_ms: Option<u64>,
    /// Time elapsed at the moment of the snapshot, from the telemetry's start.
    pub elapsed_ms: u64,
    pub first_chunk_start_ms: Option<u64>,
    pub first_chunk_ack_ms: Option<u64>,
    pub upload_start_ms: Option<u64>,
    pub upload_end_ms: Option<u64>,
    pub persistence_ms: u64,
    pub failed_chunks: u64,
    pub read_ns: u64,
    pub compress_ns: u64,
    pub encrypt_ns: u64,
    pub encrypt_cpu_ns: u64,
    pub http_ns: u64,
    /// Plaintext bytes acknowledged, and the body bytes that carried them.
    pub plaintext_bytes: u64,
    pub wire_bytes: u64,
    pub latency: PersistedLatency,
    pub read_latency: PersistedLatency,
    pub compress_latency: PersistedLatency,
    pub encrypt_latency: PersistedLatency,
    pub encrypt_cpu_latency: PersistedLatency,
    pub http_latency: PersistedLatency,
    pub files: Vec<PersistedFileTelemetry>,
    pub resources: PersistedResources,
    pub phases: PhaseTimings,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedLatency {
    pub buckets: Vec<u32>,
    pub count: u64,
    pub sum_ns: u64,
    pub max_ns: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedFileTelemetry {
    pub file_id: String,
    pub first_ms: Option<u64>,
    pub last_ms: Option<u64>,
    pub chunks: u64,
    pub bytes: u64,
    pub retries: u64,
    pub failed: u64,
    pub completed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PersistedResources {
    pub samples: u64,
    pub cpu_sum_milli_percent: u64,
    pub cpu_peak_milli_percent: u64,
    pub peak_memory_bytes: u64,
}

/// The live counters a transfer accumulates as it runs.
pub struct Telemetry {
    role: Role,
    started: Instant,
    /// False for a telemetry restored without any persisted measurement: there
    /// is no start time to measure a duration from, and a report built from it
    /// must say so rather than report the microseconds since it was created.
    started_known: bool,
    /// Time already elapsed when this telemetry was restored from a restart, so
    /// timestamps keep moving forward across one.
    offset_ns: AtomicU64,
    started_unix_ms: Option<u64>,
    first_chunk_start_ns: AtomicU64,
    first_chunk_ack_ns: AtomicU64,
    upload_start_ns: AtomicU64,
    upload_end_ns: AtomicU64,
    persistence_ns: AtomicU64,
    failed_chunks: AtomicU64,
    read_ns: AtomicU64,
    compress_ns: AtomicU64,
    encrypt_ns: AtomicU64,
    encrypt_cpu_ns: AtomicU64,
    http_ns: AtomicU64,
    plaintext_bytes: AtomicU64,
    wire_bytes: AtomicU64,
    latency: Mutex<LatencyHistogram>,
    read_latency: Mutex<LatencyHistogram>,
    compress_latency: Mutex<LatencyHistogram>,
    encrypt_latency: Mutex<LatencyHistogram>,
    encrypt_cpu_latency: Mutex<LatencyHistogram>,
    http_latency: Mutex<LatencyHistogram>,
    files: Mutex<HashMap<String, FileTelemetry>>,
    resources: Arc<ResourceSampler>,
    finalized: AtomicBool,
}

#[derive(Default, Clone)]
struct FileTelemetry {
    first_ns: u64,
    last_ns: u64,
    chunks: u64,
    bytes: u64,
    retries: u64,
    failed: u64,
    completed: bool,
}

impl Telemetry {
    pub fn new(role: Role) -> Arc<Self> {
        Self::create(role, None, true)
    }

    /// A telemetry continued from what a checkpoint persisted.
    ///
    /// The clock is offset by the elapsed time of the run being continued, so
    /// every later timestamp still lands after the earlier ones, and the
    /// measurements taken before the restart are the ones the report carries —
    /// not zeros invented at startup.
    pub fn restore(role: Role, persisted: Option<&PersistedTelemetry>) -> Arc<Self> {
        Self::create(role, persisted, persisted.is_some())
    }

    fn create(
        role: Role,
        persisted: Option<&PersistedTelemetry>,
        started_known: bool,
    ) -> Arc<Self> {
        let offset_ns = persisted
            .map(|persisted| ms_to_ns(persisted.elapsed_ms))
            .unwrap_or(0);

        let latency = persisted
            .map(|persisted| LatencyHistogram::from_persisted(&persisted.latency))
            .unwrap_or_default();

        let read_latency = persisted
            .map(|persisted| LatencyHistogram::from_persisted(&persisted.read_latency))
            .unwrap_or_default();

        let encrypt_latency = persisted
            .map(|persisted| LatencyHistogram::from_persisted(&persisted.encrypt_latency))
            .unwrap_or_default();

        let compress_latency = persisted
            .map(|persisted| LatencyHistogram::from_persisted(&persisted.compress_latency))
            .unwrap_or_default();

        let encrypt_cpu_latency = persisted
            .map(|persisted| LatencyHistogram::from_persisted(&persisted.encrypt_cpu_latency))
            .unwrap_or_default();

        let http_latency = persisted
            .map(|persisted| LatencyHistogram::from_persisted(&persisted.http_latency))
            .unwrap_or_default();

        let files = persisted
            .map(|persisted| {
                persisted
                    .files
                    .iter()
                    .map(|file| {
                        (
                            file.file_id.clone(),
                            FileTelemetry {
                                first_ns: file.first_ms.map(ms_to_ns).unwrap_or(0),
                                last_ns: file.last_ms.map(ms_to_ns).unwrap_or(0),
                                chunks: file.chunks,
                                bytes: file.bytes,
                                retries: file.retries,
                                failed: file.failed,
                                completed: file.completed,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let telemetry = Arc::new(Self {
            role,
            started: Instant::now(),
            started_known,
            offset_ns: AtomicU64::new(offset_ns),
            started_unix_ms: persisted
                .and_then(|persisted| persisted.started_unix_ms)
                .or_else(|| {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .map(|since| since.as_millis() as u64)
                }),
            first_chunk_start_ns: AtomicU64::new(
                persisted
                    .and_then(|persisted| persisted.first_chunk_start_ms)
                    .map(ms_to_ns)
                    .unwrap_or(UNSET),
            ),
            first_chunk_ack_ns: AtomicU64::new(
                persisted
                    .and_then(|persisted| persisted.first_chunk_ack_ms)
                    .map(ms_to_ns)
                    .unwrap_or(UNSET),
            ),
            upload_start_ns: AtomicU64::new(
                persisted
                    .and_then(|persisted| persisted.upload_start_ms)
                    .map(ms_to_ns)
                    .unwrap_or(UNSET),
            ),
            upload_end_ns: AtomicU64::new(
                persisted
                    .and_then(|persisted| persisted.upload_end_ms)
                    .map(ms_to_ns)
                    .unwrap_or(UNSET),
            ),
            persistence_ns: AtomicU64::new(
                persisted
                    .map(|persisted| ms_to_ns(persisted.persistence_ms))
                    .unwrap_or(0),
            ),
            failed_chunks: AtomicU64::new(
                persisted
                    .map(|persisted| persisted.failed_chunks)
                    .unwrap_or(0),
            ),
            read_ns: AtomicU64::new(persisted.map(|p| p.read_ns).unwrap_or(0)),
            compress_ns: AtomicU64::new(persisted.map(|p| p.compress_ns).unwrap_or(0)),
            encrypt_ns: AtomicU64::new(persisted.map(|p| p.encrypt_ns).unwrap_or(0)),
            encrypt_cpu_ns: AtomicU64::new(persisted.map(|p| p.encrypt_cpu_ns).unwrap_or(0)),
            http_ns: AtomicU64::new(persisted.map(|p| p.http_ns).unwrap_or(0)),
            plaintext_bytes: AtomicU64::new(persisted.map(|p| p.plaintext_bytes).unwrap_or(0)),
            wire_bytes: AtomicU64::new(persisted.map(|p| p.wire_bytes).unwrap_or(0)),
            latency: Mutex::new(latency),
            read_latency: Mutex::new(read_latency),
            compress_latency: Mutex::new(compress_latency),
            encrypt_latency: Mutex::new(encrypt_latency),
            encrypt_cpu_latency: Mutex::new(encrypt_cpu_latency),
            http_latency: Mutex::new(http_latency),
            files: Mutex::new(files),
            resources: ResourceSampler::start(),
            finalized: AtomicBool::new(false),
        });

        if let Some(persisted) = persisted {
            let resources = &persisted.resources;

            telemetry
                .resources
                .samples
                .store(resources.samples, Ordering::Relaxed);
            telemetry
                .resources
                .cpu_sum_milli_percent
                .store(resources.cpu_sum_milli_percent, Ordering::Relaxed);
            telemetry
                .resources
                .cpu_peak_milli_percent
                .store(resources.cpu_peak_milli_percent, Ordering::Relaxed);
            telemetry
                .resources
                .peak_memory_bytes
                .store(resources.peak_memory_bytes, Ordering::Relaxed);
        }

        telemetry
    }

    /// One chunk's measured phases, added to the running totals.
    ///
    /// Zero durations are skipped: a session that does not measure a phase (a
    /// test double, or a path that does not take it) leaves it absent rather
    /// than recording a fake zero sample.
    pub fn chunk_phases(
        &self,
        read: std::time::Duration,
        compress: std::time::Duration,
        encrypt: std::time::Duration,
        encrypt_cpu: std::time::Duration,
        http: std::time::Duration,
    ) {
        record_phase(&self.read_ns, &self.read_latency, read);
        record_phase(&self.compress_ns, &self.compress_latency, compress);
        record_phase(&self.encrypt_ns, &self.encrypt_latency, encrypt);
        record_phase(&self.encrypt_cpu_ns, &self.encrypt_cpu_latency, encrypt_cpu);
        record_phase(&self.http_ns, &self.http_latency, http);
    }

    /// The bytes one acknowledged chunk carried, plaintext and on the wire.
    ///
    /// Recorded only after the receiver acknowledged the chunk, so the ratio
    /// describes bytes that actually landed.
    pub fn chunk_bytes(&self, plaintext: u64, wire: u64) {
        self.plaintext_bytes.fetch_add(plaintext, Ordering::Relaxed);

        self.wire_bytes.fetch_add(wire, Ordering::Relaxed);
    }

    /// Where the chunk time went, as measured.
    fn chunk_breakdown(&self) -> ChunkBreakdown {
        let read = self.read_ns.load(Ordering::Relaxed);
        let compress = self.compress_ns.load(Ordering::Relaxed);
        let encrypt = self.encrypt_ns.load(Ordering::Relaxed);
        let encrypt_cpu = self.encrypt_cpu_ns.load(Ordering::Relaxed);
        let http = self.http_ns.load(Ordering::Relaxed);

        let total = read
            .saturating_add(compress)
            .saturating_add(encrypt)
            .saturating_add(http);

        let share = |nanos: u64| -> Option<f64> {
            (total > 0).then(|| round(nanos as f64 / total as f64 * 100.0))
        };

        let seconds =
            |nanos: u64| -> Option<f64> { (nanos > 0).then(|| round(nanos as f64 / 1e9)) };

        let dominant = if total == 0 {
            None
        } else {
            let mut phases = [
                ("read", read),
                ("compress", compress),
                ("encrypt", encrypt),
                ("http", http),
            ];

            phases.sort_by_key(|(_, nanos)| std::cmp::Reverse(*nanos));

            let (name, nanos) = phases[0];

            Some(format!(
                "{name}: {:.1}% of measured chunk time",
                nanos as f64 / total as f64 * 100.0
            ))
        };

        ChunkBreakdown {
            read: PhaseShare {
                total_seconds: seconds(read),
                share_percent: share(read),
                latency: self
                    .read_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            compress: PhaseShare {
                total_seconds: seconds(compress),
                share_percent: share(compress),
                latency: self
                    .compress_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            encrypt: PhaseShare {
                total_seconds: seconds(encrypt),
                share_percent: share(encrypt),
                latency: self
                    .encrypt_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            encrypt_cpu: PhaseShare {
                total_seconds: seconds(encrypt_cpu),
                share_percent: share(encrypt_cpu),
                latency: self
                    .encrypt_cpu_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            http: PhaseShare {
                total_seconds: seconds(http),
                share_percent: share(http),
                latency: self
                    .http_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            dominant,
        }
    }

    /// Everything measured so far, as a checkpoint writes it.
    pub fn snapshot(&self, phases: &PhaseTimings) -> PersistedTelemetry {
        let latency = self
            .latency
            .lock()
            .expect("telemetry latency lock poisoned");

        let mut files: Vec<PersistedFileTelemetry> = self
            .files
            .lock()
            .expect("telemetry files lock poisoned")
            .iter()
            .map(|(file_id, entry)| PersistedFileTelemetry {
                file_id: file_id.clone(),
                first_ms: (entry.first_ns != 0).then_some(entry.first_ns / 1_000_000),
                last_ms: (entry.last_ns != 0).then_some(entry.last_ns / 1_000_000),
                chunks: entry.chunks,
                bytes: entry.bytes,
                retries: entry.retries,
                failed: entry.failed,
                completed: entry.completed,
            })
            .collect();

        files.sort_by(|left, right| left.file_id.cmp(&right.file_id));

        PersistedTelemetry {
            started_unix_ms: self.started_unix_ms,
            elapsed_ms: self.now_ns() / 1_000_000,
            first_chunk_start_ms: optional_ms(self.first_chunk_start_ns.load(Ordering::Relaxed)),
            first_chunk_ack_ms: optional_ms(self.first_chunk_ack_ns.load(Ordering::Relaxed)),
            upload_start_ms: optional_ms(self.upload_start_ns.load(Ordering::Relaxed)),
            upload_end_ms: optional_ms(self.upload_end_ns.load(Ordering::Relaxed)),
            persistence_ms: self.persistence_ns.load(Ordering::Relaxed) / 1_000_000,
            failed_chunks: self.failed_chunks.load(Ordering::Relaxed),
            read_ns: self.read_ns.load(Ordering::Relaxed),
            compress_ns: self.compress_ns.load(Ordering::Relaxed),
            encrypt_ns: self.encrypt_ns.load(Ordering::Relaxed),
            encrypt_cpu_ns: self.encrypt_cpu_ns.load(Ordering::Relaxed),
            http_ns: self.http_ns.load(Ordering::Relaxed),
            plaintext_bytes: self.plaintext_bytes.load(Ordering::Relaxed),
            wire_bytes: self.wire_bytes.load(Ordering::Relaxed),
            latency: PersistedLatency {
                buckets: latency.buckets.to_vec(),
                count: latency.count,
                sum_ns: latency.sum_ns,
                max_ns: latency.max_ns,
            },
            read_latency: latched(&self.read_latency),
            compress_latency: latched(&self.compress_latency),
            encrypt_latency: latched(&self.encrypt_latency),
            encrypt_cpu_latency: latched(&self.encrypt_cpu_latency),
            http_latency: latched(&self.http_latency),
            files,
            resources: PersistedResources {
                samples: self.resources.samples.load(Ordering::Relaxed),
                cpu_sum_milli_percent: self.resources.cpu_sum_milli_percent.load(Ordering::Relaxed),
                cpu_peak_milli_percent: self
                    .resources
                    .cpu_peak_milli_percent
                    .load(Ordering::Relaxed),
                peak_memory_bytes: self.resources.peak_memory_bytes.load(Ordering::Relaxed),
            },
            phases: phases.clone(),
        }
    }

    fn now_ns(&self) -> u64 {
        let since_start = self.started.elapsed().as_nanos().min(u64::MAX as u128) as u64;

        self.offset_ns
            .load(Ordering::Relaxed)
            .saturating_add(since_start)
    }

    /// A chunk is about to be sent (sender) or stored (receiver).
    pub fn chunk_started(&self, file_id: &str, bytes: u64) {
        let now = self.now_ns();

        let _ = self.first_chunk_start_ns.compare_exchange(
            UNSET,
            now,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        let _ =
            self.upload_start_ns
                .compare_exchange(UNSET, now, Ordering::Relaxed, Ordering::Relaxed);

        let mut files = self.files.lock().expect("telemetry files lock poisoned");
        let entry = files
            .entry(file_id.to_owned())
            .or_insert_with(|| FileTelemetry {
                first_ns: now,
                ..FileTelemetry::default()
            });

        if entry.first_ns == 0 {
            entry.first_ns = now;
        }

        entry.bytes = entry.bytes.saturating_add(bytes);
    }

    /// A chunk was accepted, after `elapsed`.
    pub fn chunk_finished(&self, file_id: &str, elapsed: std::time::Duration, retries: u64) {
        let now = self.now_ns();

        let _ = self.first_chunk_ack_ns.compare_exchange(
            UNSET,
            now,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );

        self.upload_end_ns.store(now, Ordering::Relaxed);

        self.latency
            .lock()
            .expect("telemetry latency lock poisoned")
            .record(elapsed.as_nanos().min(u64::MAX as u128) as u64);

        let mut files = self.files.lock().expect("telemetry files lock poisoned");
        let entry = files
            .entry(file_id.to_owned())
            .or_insert_with(|| FileTelemetry {
                first_ns: now,
                ..FileTelemetry::default()
            });

        if entry.first_ns == 0 {
            entry.first_ns = now;
        }

        entry.last_ns = now;
        entry.chunks = entry.chunks.saturating_add(1);
        entry.retries = entry.retries.saturating_add(retries);
    }

    /// A chunk failed for good (all retries spent, or refused by the receiver).
    pub fn chunk_failed(&self, file_id: &str) {
        self.failed_chunks.fetch_add(1, Ordering::Relaxed);

        let now = self.now_ns();
        let mut files = self.files.lock().expect("telemetry files lock poisoned");

        let entry = files
            .entry(file_id.to_owned())
            .or_insert_with(|| FileTelemetry {
                first_ns: now,
                ..FileTelemetry::default()
            });

        entry.failed = entry.failed.saturating_add(1);
    }

    /// A file was published (receiver) or fully acknowledged (sender).
    pub fn file_completed(&self, file_id: &str) {
        if let Some(entry) = self
            .files
            .lock()
            .expect("telemetry files lock poisoned")
            .get_mut(file_id)
        {
            entry.completed = true;
        }
    }

    /// The files this telemetry has seen chunks for.
    ///
    /// The receiver never receives a file plan, so its per-file reports are
    /// built from what arrived: the ids, the bytes and the chunk counts it
    /// actually handled, with no size the sender promised and it did not get.
    pub fn file_descriptors(&self) -> Vec<FileDescriptor> {
        let mut files: Vec<FileDescriptor> = self
            .files
            .lock()
            .expect("telemetry files lock poisoned")
            .iter()
            .map(|(file_id, entry)| FileDescriptor {
                file_id: file_id.clone(),
                relative_path: None,
                size: entry.bytes,
                expected_chunks: entry.chunks,
            })
            .collect();

        files.sort_by(|left, right| left.file_id.cmp(&right.file_id));

        files
    }

    /// Time spent writing the transfer's own record, e.g. a checkpoint.
    pub fn add_persistence(&self, elapsed: std::time::Duration) {
        self.persistence_ns.fetch_add(
            elapsed.as_nanos().min(u64::MAX as u128) as u64,
            Ordering::Relaxed,
        );
    }

    /// Write the report, once.
    ///
    /// Returns `None` when this transfer has already been finalized, which is
    /// how a cancellation racing a scheduler failure still produces one report.
    pub async fn finalize(
        &self,
        reports_dir: &Path,
        finish: Finish,
        compression_enabled: bool,
        integrity_digests_enabled: bool,
    ) -> Option<ReportPaths> {
        if self
            .finalized
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return None;
        }

        self.resources.stop();

        let report = self.build(finish, compression_enabled, integrity_digests_enabled);

        match write(reports_dir, &report).await {
            Ok(paths) => {
                tracing::info!(
                    transfer_id = %report.transfer_id,
                    json = %paths.json.display(),
                    markdown = %paths.markdown.display(),
                    "transfer report written"
                );

                Some(paths)
            }

            Err(error) => {
                tracing::warn!(
                    transfer_id = %report.transfer_id,
                    %error,
                    "could not write the transfer report"
                );

                None
            }
        }
    }

    fn build(
        &self,
        finish: Finish,
        compression_enabled: bool,
        integrity_digests_enabled: bool,
    ) -> TransferReport {
        let now_ns = self.now_ns();

        // A zero duration means nothing was measured, not that the transfer was
        // instantaneous — and a telemetry restored with no persisted timing has
        // no duration to report at all.
        let duration_seconds = (self.started_known && now_ns > 0)
            .then(|| round(now_ns as f64 / 1e9))
            .filter(|seconds| *seconds > 0.0);

        let first_start = self.first_chunk_start_ns.load(Ordering::Relaxed);
        let first_ack = self.first_chunk_ack_ns.load(Ordering::Relaxed);
        let upload_start = self.upload_start_ns.load(Ordering::Relaxed);
        let upload_end = self.upload_end_ns.load(Ordering::Relaxed);

        /*
         * A window is only usable when it is positive. `upload_end >=
         * upload_start` would accept a zero-length window and turn it into a
         * division by zero dressed up as a throughput.
         */
        let window = match (upload_start == UNSET, upload_end == UNSET) {
            (false, false) if upload_end > upload_start => {
                Some((upload_end - upload_start) as f64 / 1e9)
            }

            _ => None,
        };

        let bytes = finish.completed_bytes;

        let wire_bytes = self.wire_bytes.load(Ordering::Relaxed);

        // Only the chunks this run actually sent: a resume skips the ones the
        // receiver already held, and they have no wire bytes to divide by.
        let plaintext_sent = self.plaintext_bytes.load(Ordering::Relaxed);

        let throughput = Throughput {
            bytes_acknowledged: bytes,
            denominator: "first chunk started to last chunk acknowledged",
            window_seconds: window,
            mib_s: window.map(|seconds| mib_per_second(bytes, seconds)),
            gib_s: window.map(|seconds| mib_per_second(bytes, seconds) / 1024.0),
            wire_bytes,
            wire_mib_s: window.map(|seconds| mib_per_second(wire_bytes, seconds)),
            compression_ratio: (wire_bytes > 0)
                .then(|| round(plaintext_sent as f64 / wire_bytes as f64)),
        };

        let mut files = Vec::with_capacity(finish.files.len());
        let known = (*self.files.lock().expect("telemetry files lock poisoned")).clone();

        for descriptor in &finish.files {
            let observed = known.get(&descriptor.file_id);

            let duration_seconds = observed.and_then(|entry| {
                (entry.last_ns > entry.first_ns && entry.first_ns != 0)
                    .then_some((entry.last_ns - entry.first_ns) as f64 / 1e9)
            });

            let bytes_done = observed.map(|entry| entry.bytes).unwrap_or(0);
            let chunks_done = observed.map(|entry| entry.chunks).unwrap_or(0);
            /*
             * Success is only ever what the receiver confirmed by publishing.
             * Every chunk being acknowledged is not that: the last one may be
             * accepted while the file is still being assembled, and this side
             * never sees the publication. So an acknowledged file this side has
             * no confirmation for is `unknown`, not `completed` and not a
             * failure.
             */
            let acknowledged = chunks_done >= descriptor.expected_chunks;

            let outcome = if observed.map(|entry| entry.completed).unwrap_or(false) {
                "completed"
            } else if acknowledged {
                "unknown"
            } else {
                "incomplete"
            };

            files.push(FileReport {
                file_id: descriptor.file_id.clone(),
                relative_path: descriptor.relative_path.clone(),
                size_bytes: descriptor.size,
                chunks: descriptor.expected_chunks,
                chunks_done,
                bytes_done,
                retries: observed.map(|entry| entry.retries).unwrap_or(0),
                failed_chunks: observed.map(|entry| entry.failed).unwrap_or(0),
                duration_seconds,
                throughput_mib_s: duration_seconds
                    .map(|seconds| mib_per_second(bytes_done, seconds)),
                outcome,
            });
        }

        let completed_files = files
            .iter()
            .filter(|file| file.outcome == "completed")
            .count() as u64;

        let unknown_files = files
            .iter()
            .filter(|file| file.outcome == "unknown")
            .count() as u64;

        let mut phases = finish.phases.clone();
        phases.upload_seconds = window;
        phases.chunk_persistence_seconds = match self.persistence_ns.load(Ordering::Relaxed) {
            0 => phases.chunk_persistence_seconds,
            nanos => Some(nanos as f64 / 1e9),
        };

        let chunk_breakdown = self.chunk_breakdown();

        let mut unavailable = finish.unavailable.clone();

        if first_start == UNSET {
            unavailable.push("first_chunk_start_seconds".into());
        }

        if first_ack == UNSET {
            unavailable.push("first_chunk_ack_seconds".into());
        }

        if window.is_none() {
            unavailable.push("throughput (no positive acknowledged-chunk window)".into());
        }

        if wire_bytes == 0 {
            unavailable
                .push("wire throughput and compression ratio (no body bytes recorded)".into());
        }

        if duration_seconds.is_none() {
            unavailable.push("duration (no measured elapsed time)".into());
        }

        if self.resources.summary().samples == 0 {
            unavailable.push("resources (no process sample)".into());
        }

        if chunk_breakdown.dominant.is_none() {
            unavailable.push("chunk_breakdown (no per-phase chunk timings)".into());
        }

        TransferReport {
            schema_version: REPORT_SCHEMA_VERSION,
            role: self.role,
            transfer_id: finish.transfer_id.clone(),
            status: status_name(finish.status).to_owned(),
            started_at_unix_ms: self.started_unix_ms,
            finished_at_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_millis() as u64)
                .unwrap_or_default(),
            duration_seconds,
            total_files: finish.files.len() as u64,
            completed_files,
            unknown_files,
            remaining_files: finish.files.len() as u64 - completed_files - unknown_files,
            total_bytes: finish.total_bytes,
            completed_bytes: finish.completed_bytes,
            remaining_bytes: finish.total_bytes.saturating_sub(finish.completed_bytes),
            total_chunks: finish.total_chunks,
            completed_chunks: finish.completed_chunks,
            throughput,
            first_chunk_start_seconds: (first_start != UNSET)
                .then_some(round(first_start as f64 / 1e9)),
            first_chunk_ack_seconds: (first_ack != UNSET).then_some(round(first_ack as f64 / 1e9)),
            chunk_latency: self
                .latency
                .lock()
                .expect("telemetry latency lock poisoned")
                .summary(),
            chunk_breakdown,
            resources: self.resources.summary(),
            phases,
            retries: finish.retries,
            failed_chunks: self.failed_chunks.load(Ordering::Relaxed),
            failure_stage: finish.failure_stage,
            failure_reason: finish.failure_reason,
            cancellation_reason: finish.cancellation_reason,
            files,
            compression_enabled,
            integrity_digests_enabled,
            unavailable,
            recovered: finish.recovered,
            notes: {
                let mut notes: Vec<String> = vec![
                    "throughput counts acknowledged plaintext bytes over the upload window; it is not link bandwidth".into(),
                    "wire_bytes are chunk body bytes after compression (no HTTP headers); compare wire_mib_s with the link, and use compression_ratio to see what compression removed".into(),
                    "CPU and memory are process-wide, so concurrent transfers share them".into(),
                    "CPU percentages are of one core: a value above 100 means more than one core was busy (peak / 100 is roughly the core count)".into(),
                    "percentiles are log2 bucket upper bounds, within a factor of two".into(),
                    "first_chunk_start_seconds is when the first chunk's work began (read, seal, send), not when its first byte left the machine".into(),
                    "a file is completed only when the receiver published it; every chunk acknowledged without that confirmation is reported as unknown".into(),
                ];

                if finish.recovered {
                    notes.push(
                        "recovered after a restart: the values are the last checkpoint's, and no time the process was not running is counted".into(),
                    );
                }

                notes
            },
        }
    }
}

fn ms_to_ns(millis: u64) -> u64 {
    millis.saturating_mul(1_000_000)
}

fn record_phase(
    total: &AtomicU64,
    latency: &Mutex<LatencyHistogram>,
    elapsed: std::time::Duration,
) {
    let nanos = elapsed.as_nanos().min(u64::MAX as u128) as u64;

    if nanos == 0 {
        return;
    }

    total.fetch_add(nanos, Ordering::Relaxed);

    latency
        .lock()
        .expect("telemetry latency lock poisoned")
        .record(nanos);
}

/// A histogram as the checkpoint writes it.
fn latched(latency: &Mutex<LatencyHistogram>) -> PersistedLatency {
    let histogram = latency.lock().expect("telemetry latency lock poisoned");

    PersistedLatency {
        buckets: histogram.buckets.to_vec(),
        count: histogram.count,
        sum_ns: histogram.sum_ns,
        max_ns: histogram.max_ns,
    }
}

fn optional_ms(nanos: u64) -> Option<u64> {
    (nanos != UNSET).then_some(nanos / 1_000_000)
}

fn status_name(status: TransferStatus) -> &'static str {
    match status {
        TransferStatus::Completed => "COMPLETED",
        TransferStatus::Failed => "FAILED",
        TransferStatus::Cancelled => "CANCELLED",
        TransferStatus::Pending => "PENDING",
        TransferStatus::Queued => "QUEUED",
        TransferStatus::Uploading => "UPLOADING",
        TransferStatus::Downloading => "DOWNLOADING",
        TransferStatus::Paused => "PAUSED",
        _ => "UNKNOWN",
    }
}

fn mib_per_second(bytes: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }

    round(bytes as f64 / (1024.0 * 1024.0) / seconds)
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Where a finished report was written.
#[derive(Debug, Clone)]
pub struct ReportPaths {
    pub json: PathBuf,
    pub markdown: PathBuf,
}

/// Where a transfer's report lives.
///
/// The transfer id is sanitised to a file name, so an id from the control plane
/// cannot name a path outside the reports directory.
pub fn report_path(reports_dir: &Path, transfer_id: &str) -> PathBuf {
    let safe_id: String = transfer_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect();

    reports_dir.join(format!("{safe_id}.json"))
}

/// Write one report as JSON and Markdown, atomically.
pub async fn write(reports_dir: &Path, report: &TransferReport) -> io::Result<ReportPaths> {
    tokio::fs::create_dir_all(reports_dir).await?;

    let json_path = report_path(reports_dir, &report.transfer_id);

    let markdown_path = json_path.with_extension("md");

    let json = serde_json::to_vec_pretty(report).map_err(io::Error::other)?;

    crate::transfer::atomic_file::write_atomic(&json_path, &json).await?;

    crate::transfer::atomic_file::write_atomic(&markdown_path, render_markdown(report).as_bytes())
        .await?;

    Ok(ReportPaths {
        json: json_path,
        markdown: markdown_path,
    })
}

/// The human-readable form. Same facts as the JSON, in the order a person reads
/// them.
pub fn render_markdown(report: &TransferReport) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "# Transfer {} — {}\n\n",
        report.transfer_id, report.status
    ));

    out.push_str(&format!(
        "- role: {}\n- schema: {}\n- duration: {}\n- bytes: {} of {} acknowledged ({} remaining)\n- chunks: {} of {}\n- files: {} completed, {} unknown, {} total\n- compression: {}\n- integrity digests: {}\n- recovered after restart: {}\n\n",
        match report.role {
            Role::Sender => "sender",
            Role::Receiver => "receiver",
        },
        report.schema_version,
        format_option(report.duration_seconds, "s"),
        report.completed_bytes,
        report.total_bytes,
        report.remaining_bytes,
        report.completed_chunks,
        report.total_chunks,
        report.completed_files,
        report.unknown_files,
        report.total_files,
        report.compression_enabled,
        report.integrity_digests_enabled,
        report.recovered,
    ));

    out.push_str("## Throughput\n\n");
    out.push_str(&format!(
        "| metric | value |\n|---|---|\n| acknowledged (plaintext) | {} |\n| wire (after compression) | {} |\n| compression ratio | {} |\n| window | {} |\n| MiB/s (plaintext) | {} |\n| MiB/s (wire) | {} |\n| GiB/s (plaintext) | {} |\n\n",
        format_bytes(report.throughput.bytes_acknowledged),
        format_bytes(report.throughput.wire_bytes),
        format_option(report.throughput.compression_ratio, "x"),
        report.throughput.denominator,
        format_option(report.throughput.mib_s, "MiB/s"),
        format_option(report.throughput.wire_mib_s, "MiB/s"),
        format_option(report.throughput.gib_s, "GiB/s"),
    ));

    out.push_str("## Latency\n\n");
    out.push_str(&format!(
        "| metric | value |\n|---|---|\n| chunks | {} |\n| p50 | {} |\n| p95 | {} |\n| p99 | {} |\n| max | {} |\n\n",
        report.chunk_latency.count,
        format_option(report.chunk_latency.p50_ms, "ms"),
        format_option(report.chunk_latency.p95_ms, "ms"),
        format_option(report.chunk_latency.p99_ms, "ms"),
        format_option(report.chunk_latency.max_ms, "ms"),
    ));

    out.push_str("## Phases\n\n| phase | seconds |\n|---|---|\n");

    let phases = [
        ("initialization", report.phases.initialization_seconds),
        ("handshake", report.phases.handshake_seconds),
        ("upload", report.phases.upload_seconds),
        ("chunk persistence", report.phases.chunk_persistence_seconds),
        ("final assembly", report.phases.final_assembly_seconds),
        ("verification", report.phases.verification_seconds),
        ("publication", report.phases.publication_seconds),
    ];

    for (name, seconds) in phases {
        out.push_str(&format!("| {name} | {} |\n", format_option(seconds, "s")));
    }

    out.push_str("\n## Resources\n\n");
    out.push_str(&format!(
        "| metric | value |\n|---|---|\n| samples | {} |\n| CPU average | {} |\n| CPU peak | {} |\n| memory peak | {} |\n\n",
        report.resources.samples,
        format_option(report.resources.cpu_average_percent, "% of one core"),
        format_option(report.resources.cpu_peak_percent, "% of one core"),
        report
            .resources
            .memory_peak_bytes
            .map(format_bytes)
            .unwrap_or_else(|| "n/a".into()),
    ));

    if let Some(stage) = &report.failure_stage {
        out.push_str(&format!("## Failure\n\n- stage: {stage}\n"));

        if let Some(reason) = &report.failure_reason {
            out.push_str(&format!("- reason: {reason}\n"));
        }

        out.push('\n');
    }

    if let Some(reason) = &report.cancellation_reason {
        out.push_str(&format!("## Cancellation\n\n- reason: {reason}\n\n"));
    }

    if !report.files.is_empty() {
        out.push_str("## Files\n\n| file | size | chunks | retries | throughput | outcome |\n|---|---|---|---|---|---|\n");

        for file in &report.files {
            out.push_str(&format!(
                "| {} | {} | {}/{} | {} | {} | {} |\n",
                file.relative_path
                    .clone()
                    .unwrap_or_else(|| file.file_id.clone()),
                format_bytes(file.size_bytes),
                file.chunks_done,
                file.chunks,
                file.retries,
                format_option(file.throughput_mib_s, "MiB/s"),
                file.outcome,
            ));
        }

        out.push('\n');
    }

    if !report.unavailable.is_empty() {
        out.push_str("## Not measured\n\n");

        for item in &report.unavailable {
            out.push_str(&format!("- {item}\n"));
        }

        out.push('\n');
    }

    out.push_str("## Notes\n\n");

    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }

    out
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

    let mut value = bytes as f64;
    let mut unit = 0;

    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    format!("{:.2} {}", value, UNITS[unit])
}

fn format_option(value: Option<f64>, suffix: &str) -> String {
    match value {
        Some(value) => format!("{value:.3} {suffix}"),
        None => "n/a".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::manifest::test_support::TempDir;

    fn finish(transfer_id: &str, status: TransferStatus) -> Finish {
        Finish {
            transfer_id: transfer_id.into(),
            status,
            total_bytes: 100,
            completed_bytes: 100,
            total_chunks: 2,
            completed_chunks: 2,
            files: vec![FileDescriptor {
                file_id: "f-1".into(),
                relative_path: Some("a.bin".into()),
                size: 100,
                expected_chunks: 2,
            }],
            retries: 1,
            failure_stage: None,
            failure_reason: None,
            cancellation_reason: None,
            phases: PhaseTimings::default(),
            unavailable: Vec::new(),
            recovered: false,
        }
    }

    #[tokio::test]
    async fn a_completed_transfer_writes_json_and_markdown() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");

        let telemetry = Telemetry::new(Role::Sender);

        telemetry.chunk_started("f-1", 50);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(20), 1);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(10), 0);

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("the report is written");

        assert!(paths.json.exists());
        assert!(paths.markdown.exists());

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["transfer_id"], "t-1");
        assert_eq!(json["status"], "COMPLETED");
        assert_eq!(json["role"], "sender");
        assert_eq!(json["compression_enabled"], false);
        assert_eq!(json["integrity_digests_enabled"], false);
        assert_eq!(json["chunk_latency"]["count"], 2);
        assert!(json["chunk_latency"]["p50_ms"].is_number());

        let markdown = std::fs::read_to_string(&paths.markdown).expect("reading");

        assert!(markdown.contains("Throughput"));
    }

    #[tokio::test]
    async fn a_transfer_is_reported_exactly_once() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        let first = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await;

        let second = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Failed),
                false,
                false,
            )
            .await;

        assert!(first.is_some(), "the first terminal path writes the report");
        assert!(second.is_none(), "a racing second path must not");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(report_path(&reports, "t-1")).expect("reading"))
                .expect("json");

        assert_eq!(
            json["status"], "COMPLETED",
            "the report is the first outcome, not the last one to arrive"
        );
    }

    #[tokio::test]
    async fn a_failure_records_its_stage_and_reason() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        let mut finish = finish("t-1", TransferStatus::Failed);
        finish.failure_stage = Some("upload".into());
        finish.failure_reason = Some("receiver refused every retry".into());

        let paths = telemetry
            .finalize(&reports, finish, false, false)
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["failure_stage"], "upload");
        assert_eq!(json["failure_reason"], "receiver refused every retry");
    }

    #[tokio::test]
    async fn a_cancellation_records_its_reason() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        let mut finish = finish("t-1", TransferStatus::Cancelled);
        finish.cancellation_reason = Some("cancelled by the user".into());

        let paths = telemetry
            .finalize(&reports, finish, false, false)
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["status"], "CANCELLED");
        assert_eq!(json["cancellation_reason"], "cancelled by the user");
    }

    #[tokio::test]
    async fn a_transfer_with_no_chunks_marks_its_timings_unavailable() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        let mut finish = finish("t-empty", TransferStatus::Failed);
        finish.total_bytes = 0;
        finish.completed_bytes = 0;
        finish.total_chunks = 0;
        finish.completed_chunks = 0;
        finish.files.clear();

        let paths = telemetry
            .finalize(&reports, finish, false, false)
            .await
            .expect("a zero-byte transfer is still reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert!(json["first_chunk_start_seconds"].is_null());
        assert!(json["throughput"]["mib_s"].is_null());

        let unavailable = json["unavailable"].as_array().expect("unavailable");

        assert!(unavailable
            .iter()
            .any(|item| item == "first_chunk_start_seconds"));
    }

    #[tokio::test]
    async fn retries_are_recorded_per_file() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        telemetry.chunk_started("f-1", 50);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(1), 3);

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["files"][0]["retries"], 3);
    }

    #[tokio::test]
    async fn concurrent_transfers_get_their_own_reports() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");

        let first = Telemetry::new(Role::Sender);
        let second = Telemetry::new(Role::Receiver);

        let (a, b) = tokio::join!(
            first.finalize(
                &reports,
                finish("t-a", TransferStatus::Completed),
                false,
                false
            ),
            second.finalize(
                &reports,
                finish("t-b", TransferStatus::Failed),
                false,
                false
            ),
        );

        assert!(a.is_some());
        assert!(b.is_some());
        assert!(report_path(&reports, "t-a").exists());
        assert!(report_path(&reports, "t-b").exists());
    }

    /*
     * ----------------------------------------------------------------------
     * Acknowledged is not published
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn an_acknowledged_file_without_publication_is_unknown_not_complete() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        telemetry.chunk_started("f-1", 50);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(5), 0);
        telemetry.chunk_started("f-1", 50);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(5), 0);

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["files"][0]["chunks_done"], 2);
        assert_eq!(
            json["files"][0]["outcome"], "unknown",
            "both chunks were acknowledged, publication was not confirmed here"
        );
        assert_eq!(json["completed_files"], 0);
        assert_eq!(json["unknown_files"], 1);
        assert_eq!(json["remaining_files"], 0);
    }

    #[tokio::test]
    async fn a_published_file_is_complete() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Receiver);

        telemetry.chunk_started("f-1", 100);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(5), 0);
        telemetry.file_completed("f-1");

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["files"][0]["outcome"], "completed");
        assert_eq!(json["completed_files"], 1);
        assert_eq!(json["unknown_files"], 0);
    }

    /*
     * ----------------------------------------------------------------------
     * Durations and windows
     * ----------------------------------------------------------------------
     */

    #[tokio::test]
    async fn a_zero_length_upload_window_reports_no_throughput() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");

        // A window that survives persistence as `start == end` — the shape that
        // a division would otherwise turn into an infinite throughput.
        let persisted = PersistedTelemetry {
            elapsed_ms: 1_000,
            persistence_ms: 0,
            upload_start_ms: Some(500),
            upload_end_ms: Some(500),
            ..PersistedTelemetry::default()
        };

        let telemetry = Telemetry::restore(Role::Sender, Some(&persisted));

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert!(json["throughput"]["window_seconds"].is_null());
        assert!(json["throughput"]["mib_s"].is_null());
        assert!(json["throughput"]["gib_s"].is_null());
    }

    #[tokio::test]
    async fn a_restored_telemetry_keeps_the_run_it_measured() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");

        let persisted = PersistedTelemetry {
            started_unix_ms: Some(1_700_000_000_000),
            elapsed_ms: 12_000,
            persistence_ms: 0,
            read_ns: 3_000_000_000,
            compress_ns: 500_000_000,
            encrypt_ns: 1_000_000_000,
            encrypt_cpu_ns: 600_000_000,
            http_ns: 6_000_000_000,
            plaintext_bytes: 100,
            wire_bytes: 40,
            read_latency: PersistedLatency {
                count: 2,
                sum_ns: 3_000_000_000,
                max_ns: 2_000_000_000,
                ..PersistedLatency::default()
            },
            compress_latency: PersistedLatency::default(),
            encrypt_latency: PersistedLatency::default(),
            encrypt_cpu_latency: PersistedLatency::default(),
            http_latency: PersistedLatency::default(),
            first_chunk_start_ms: Some(250),
            first_chunk_ack_ms: Some(400),
            upload_start_ms: Some(250),
            upload_end_ms: Some(11_000),
            failed_chunks: 1,
            latency: PersistedLatency {
                count: 2,
                sum_ns: 2_000_000,
                max_ns: 1_500_000,
                ..PersistedLatency::default()
            },
            files: vec![PersistedFileTelemetry {
                file_id: "f-1".into(),
                first_ms: Some(250),
                last_ms: Some(11_000),
                chunks: 2,
                bytes: 100,
                retries: 1,
                failed: 0,
                completed: false,
            }],
            resources: PersistedResources {
                samples: 20,
                cpu_sum_milli_percent: 40_000,
                cpu_peak_milli_percent: 9_000,
                peak_memory_bytes: 8 * 1024 * 1024,
            },
            phases: PhaseTimings {
                initialization_seconds: Some(0.25),
                ..PhaseTimings::default()
            },
        };

        let telemetry = Telemetry::restore(Role::Sender, Some(&persisted));

        let mut finish = finish("t-1", TransferStatus::Completed);
        finish.recovered = true;

        let paths = telemetry
            .finalize(&reports, finish, false, false)
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["recovered"], true);
        assert_eq!(json["started_at_unix_ms"], 1_700_000_000_000u64);
        assert!(
            json["duration_seconds"].as_f64().unwrap() >= 12.0,
            "the persisted elapsed time is the duration, not the microseconds since restore"
        );
        assert!(json["throughput"]["mib_s"].is_number());
        assert_eq!(
            json["phases"]["upload_seconds"].as_f64().unwrap().round(),
            11.0
        );
        assert_eq!(json["first_chunk_start_seconds"], 0.25);
        assert_eq!(json["first_chunk_ack_seconds"], 0.4);
        assert_eq!(json["files"][0]["chunks_done"], 2);
        assert_eq!(json["files"][0]["retries"], 1);
        assert_eq!(json["files"][0]["outcome"], "unknown");
        assert_eq!(json["chunk_latency"]["count"], 2);
        assert_eq!(json["resources"]["samples"], 20);
        assert_eq!(json["resources"]["memory_peak_bytes"], 8 * 1024 * 1024);
        assert_eq!(json["failed_chunks"], 1);

        // The chunk-time attribution survives a restart too.
        assert_eq!(json["chunk_breakdown"]["read"]["total_seconds"], 3.0);
        assert_eq!(json["chunk_breakdown"]["read"]["share_percent"], 28.571);
        assert_eq!(json["chunk_breakdown"]["http"]["share_percent"], 57.143);
        assert!(json["chunk_breakdown"]["dominant"]
            .as_str()
            .unwrap()
            .starts_with("http:"));
        assert_eq!(json["chunk_breakdown"]["read"]["latency"]["count"], 2);
    }

    #[tokio::test]
    async fn a_restored_telemetry_with_nothing_persisted_has_no_duration() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");

        let telemetry = Telemetry::restore(Role::Sender, None);

        let mut finish = finish("t-1", TransferStatus::Failed);
        finish.recovered = true;

        let paths = telemetry
            .finalize(&reports, finish, false, false)
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert!(json["duration_seconds"].is_null());
        assert!(json["throughput"]["mib_s"].is_null());
        assert!(json["first_chunk_start_seconds"].is_null());

        let unavailable = json["unavailable"].as_array().expect("unavailable");

        assert!(unavailable
            .iter()
            .any(|item| item.as_str().unwrap_or_default().starts_with("duration")));
    }

    #[test]
    fn a_percentile_never_exceeds_the_observed_maximum() {
        let mut histogram = LatencyHistogram::default();

        /*
         * The regression: one sample of 100.3 s landed in the bucket whose upper
         * bound is 137.4 s, and the summary reported a p50 larger than the max.
         */
        histogram.record(100_303_835_000);

        let summary = histogram.summary();

        let p50 = summary.p50_ms.expect("a sample");
        let p95 = summary.p95_ms.expect("a sample");
        let p99 = summary.p99_ms.expect("a sample");
        let max = summary.max_ms.expect("a sample");

        assert!(p50 <= max, "p50 {p50} must not exceed max {max}");
        assert!(p95 <= max);
        assert!(p99 <= max);
        assert_eq!(p50, max, "the only sample is the max");
    }

    #[test]
    fn percentiles_are_monotonic_across_buckets() {
        let mut histogram = LatencyHistogram::default();

        for nanos in [
            1_000,
            5_000,
            20_000,
            90_000,
            400_000,
            3_000_000,
            20_000_000,
            100_303_835_000,
        ] {
            histogram.record(nanos);
        }

        let summary = histogram.summary();

        let p50 = summary.p50_ms.unwrap();
        let p95 = summary.p95_ms.unwrap();
        let p99 = summary.p99_ms.unwrap();
        let max = summary.max_ms.unwrap();

        assert!(p50 <= p95, "p50 {p50} > p95 {p95}");
        assert!(p95 <= p99, "p95 {p95} > p99 {p99}");
        assert!(p99 <= max, "p99 {p99} > max {max}");
    }

    #[tokio::test]
    async fn chunk_time_is_attributed_to_the_phase_that_holds_it() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        telemetry.chunk_started("f-1", 50);
        telemetry.chunk_phases(
            std::time::Duration::from_millis(10),
            std::time::Duration::from_millis(0),
            std::time::Duration::from_millis(2),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(8),
        );
        telemetry.chunk_bytes(50, 10);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(25), 0);

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        let breakdown = &json["chunk_breakdown"];

        assert_eq!(breakdown["read"]["total_seconds"], 0.01);
        assert_eq!(breakdown["encrypt"]["total_seconds"], 0.002);
        assert_eq!(breakdown["http"]["total_seconds"], 0.008);
        assert_eq!(breakdown["read"]["share_percent"], 50.0);
        assert_eq!(breakdown["http"]["share_percent"], 40.0);

        assert_eq!(json["throughput"]["wire_bytes"], 10);
        assert_eq!(json["throughput"]["compression_ratio"], 5.0);

        assert!(
            breakdown["dominant"].as_str().unwrap().starts_with("read:"),
            "the largest measured share names the bottleneck: {}",
            breakdown["dominant"]
        );

        // The unaccounted time is the gap between the dispatch-to-ACK latency
        // and the three measured phases: backoff and scheduling, not invented.
        assert_eq!(json["chunk_latency"]["count"], 1);
    }

    #[tokio::test]
    async fn a_transfer_with_no_phase_timings_says_so() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        telemetry.chunk_started("f-1", 50);
        telemetry.chunk_finished("f-1", std::time::Duration::from_millis(1), 0);

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert!(json["chunk_breakdown"]["dominant"].is_null());

        let unavailable = json["unavailable"].as_array().expect("unavailable");

        assert!(unavailable.iter().any(|item| item
            .as_str()
            .unwrap_or_default()
            .starts_with("chunk_breakdown")));
    }

    #[tokio::test]
    async fn cpu_percentages_carry_the_core_count() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("reported");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert!(json["resources"]["logical_cores"].as_u64().unwrap() >= 1);

        let notes = json["notes"].as_array().expect("notes");

        assert!(notes.iter().any(|note| note
            .as_str()
            .unwrap_or_default()
            .contains("above 100 means more than one core")));
    }

    #[test]
    fn percentile_buckets_are_bounded_and_ordered() {
        let mut histogram = LatencyHistogram::default();

        for nanos in [1_000u64, 2_000, 3_000, 4_000, 100_000] {
            histogram.record(nanos);
        }

        let summary = histogram.summary();

        assert_eq!(summary.count, 5);
        assert!(summary.p50_ms.unwrap() <= summary.p95_ms.unwrap());
        assert!(summary.p95_ms.unwrap() <= summary.p99_ms.unwrap());
        assert!(summary.max_ms.unwrap() >= 0.1);
    }

    #[test]
    fn an_empty_histogram_has_no_percentiles() {
        let summary = LatencyHistogram::default().summary();

        assert_eq!(summary.count, 0);
        assert!(summary.p50_ms.is_none());
    }
}
