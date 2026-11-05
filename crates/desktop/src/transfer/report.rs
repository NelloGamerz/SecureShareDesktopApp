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
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use sysinfo::{Pid, ProcessesToUpdate, System};

/// The report format this build writes.
pub const REPORT_SCHEMA_VERSION: u32 = 3;

/// How often the sender samples the transfer for the `timeseries`.
pub const SAMPLE_INTERVAL_SECS: u64 = 5;

/// The most samples a `timeseries` may hold. Past this the sampler keeps every
/// second sample and doubles its interval, so a very long transfer is
/// downsampled rather than unbounded.
const MAX_TIMESERIES_SAMPLES: usize = 720;

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

/// Percentiles of a count rather than a duration, in whatever units the values
/// were recorded with. Used for the journal batch size.
#[derive(Debug, Clone, Serialize)]
pub struct CountSummary {
    pub count: u64,
    pub mean: Option<f64>,
    pub p50: Option<u64>,
    pub p95: Option<u64>,
    pub max: Option<u64>,
    pub resolution: &'static str,
}

/// One interval of a transfer's life, sampled by the sender.
///
/// The receiver columns come from its per-chunk response headers: each chunk
/// ack refreshes the latest values, and the sampler folds the newest into its
/// window. `null` means the receiver is an older build that sent no headers.
#[derive(Debug, Clone, Serialize)]
pub struct TimeSample {
    /// Seconds since the transfer started.
    pub t_secs: f64,
    pub bytes_acked: u64,
    pub chunks_acked: u64,
    /// Throughput in this interval only: `delta_bytes / interval`.
    pub window_mib_s: f64,
    /// Chunks dispatched and not yet acknowledged, at sample time.
    pub inflight_now: u32,
    /// This process, from the resource sampler's last 500 ms read.
    pub sender_rss_bytes: u64,
    pub sender_open_fds: Option<u32>,
    /// Not measured on the receiver (a header would cost a sysinfo refresh per
    /// chunk); always `null` today.
    pub recv_rss_bytes: Option<u64>,
    pub recv_inflight_map_len: Option<u32>,
    pub recv_journal_pending: Option<u32>,
    /// Journal flushes the receiver performed in this window.
    pub recv_journal_flushes_this_window: Option<u32>,
    /// Chunks in the receiver's in-memory manifest at sample time.
    pub recv_manifest_chunk_count: Option<u32>,
    pub recv_part_files: Option<u32>,
    pub recv_incoming_dir_bytes: Option<u64>,
    pub compactions_this_window: Option<u32>,
    pub avg_chunk_latency_ms_this_window: f64,
    /// Set when this one sample could not be collected; the transfer goes on.
    pub error: Option<String>,
}

/// Is this decay real, and when did it start? Derived from the timeseries.
#[derive(Debug, Clone, Serialize)]
pub struct ThroughputDecay {
    pub first_quartile_mib_s: f64,
    pub last_quartile_mib_s: f64,
    /// `first / last`; above 1.3 means real decay. Floored at 1 MiB/s so a
    /// complete stall stays a finite JSON number.
    pub decay_ratio: f64,
    /// First sample below 70% of the first quartile, or `null`.
    pub slowdown_onset_secs: Option<f64>,
}

/// The newest receiver counters seen on an ack. Internal: the report carries
/// them inside [`TimeSample`], not as a separate block.
#[derive(Debug, Clone, Default)]
pub struct ReceiverSnapshot {
    pub inflight: Option<u32>,
    pub journal_pending: Option<u32>,
    pub part_files: Option<u32>,
    pub incoming_bytes: Option<u64>,
    /// Cumulative; the sampler reports the window delta.
    pub compactions: Option<u64>,
    /// Cumulative; the sampler reports the window delta.
    pub journal_flushes: Option<u64>,
    /// Cumulative journal entries, for entries-per-flush on the final report.
    pub journal_entries: Option<u64>,
    /// Cumulative manifest-write milliseconds, total and under the lock.
    pub compaction_ms: Option<u64>,
    pub compaction_under_lock_ms: Option<u64>,
    /// The receiver's in-memory manifest at the latest acknowledgement.
    pub manifest_chunks: Option<u32>,
    pub manifest_lookup_p50_us: Option<u64>,
    pub manifest_lookup_p95_us: Option<u64>,
    pub manifest_memory_bytes: Option<u64>,
}

/// The sampled series and the interval it is being taken at.
struct TimeseriesState {
    samples: Vec<TimeSample>,
    interval_secs: u64,
}

/// Releases one in-flight slot when the chunk task ends, however it ends.
pub struct InFlightGuard(Arc<Telemetry>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.inflight_now.fetch_sub(1, Ordering::Relaxed);
    }
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

    /// Mean and percentiles in the units the samples were recorded with.
    ///
    /// `summary` talks in milliseconds because durations are what it is for;
    /// this is the same bucket walk without the unit conversion, used for the
    /// journal batch size, which is a count of entries.
    pub fn raw_summary(&self) -> CountSummary {
        if self.count == 0 {
            return CountSummary {
                count: 0,
                mean: None,
                p50: None,
                p95: None,
                max: None,
                resolution: "log2 buckets, clamped to the observed maximum",
            };
        }

        let p50 = self.percentile(50.0).min(self.max_ns);
        let p95 = self.percentile(95.0).min(self.max_ns).max(p50);

        CountSummary {
            count: self.count,
            mean: Some(round(self.sum_ns as f64 / self.count as f64)),
            p50: Some(p50),
            p95: Some(p95),
            max: Some(self.max_ns),
            resolution: "log2 buckets, clamped to the observed maximum",
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

/// Average nanoseconds spent per acknowledged chunk since `last`, advanced in
/// place. `0.0` for a window with no acknowledgements.
fn window_average_ms(total_ns: u64, last: &mut u64, chunks: u64) -> f64 {
    let delta = total_ns.saturating_sub(*last);
    *last = total_ns;

    if chunks == 0 {
        return 0.0;
    }

    round(delta as f64 / chunks as f64 / 1e6)
}

/// Process CPU and memory, sampled while the transfer runs.
#[derive(Default)]
pub struct ResourceSampler {
    /// The most recent 500 ms read, for the per-interval timeseries.
    current_memory_bytes: AtomicU64,
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

                inner
                    .current_memory_bytes
                    .store(memory, Ordering::Relaxed);
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

    /// The last sampled RSS of this process, in bytes.
    fn current_rss(&self) -> u64 {
        self.current_memory_bytes.load(Ordering::Relaxed)
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
    /// Whether compression was attempted for this file.
    pub compression_enabled: bool,
    /// Why compression was skipped, when it was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compression_skipped_reason: Option<&'static str>,
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
    /// The parts of `http` a receiver reported: how long a chunk waited for
    /// the per-transfer lock, how long its bytes took to be written and synced,
    /// how long the journal append took, and the whole durability step. All
    /// zero-count when the receiver sent no timing headers.
    pub recv_lock_wait: PhaseShare,
    pub recv_part_write: PhaseShare,
    pub recv_journal: PhaseShare,
    pub recv_accept: PhaseShare,
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
    /// The four receiver-reported parts of `recv_accept` as a fraction of the
    /// sender's `http` time. `null` when fewer than 10% of acknowledged chunks
    /// carried receiver timings — there is nothing to attribute.
    pub recv_durability_share_percent: Option<f64>,
    /// The share of acknowledged chunks whose `200` carried any receiver
    /// timing header. `null` when no chunk was acknowledged.
    pub recv_timings_coverage_percent: Option<f64>,
    /// How many journal flushes the receiver performed. One flush covers up to
    /// `JOURNAL_BATCH_ENTRIES` chunks.
    pub recv_journal_flush_count: u64,
    /// Entries per flush, `null` when there was no flush.
    pub recv_journal_entries_per_flush: Option<CountSummary>,
    /// How many times the receiver wrote its manifest.
    pub recv_compaction_count: u64,
    /// Total time the receiver spent writing its manifest, `null` when it
    /// reported nothing (a sender whose receiver sent no headers).
    pub recv_compaction_total_seconds: Option<f64>,
    /// The part of that time spent holding the transfer lock. On the per-chunk
    /// path this should be ~0; a regression shows up here.
    pub recv_compaction_under_lock_ms: Option<f64>,
    /// Chunks in the receiver's in-memory manifest at its last acknowledgement.
    pub recv_manifest_chunk_count: Option<u32>,
    /// Manifest lookup/insert percentiles, in microseconds.
    pub recv_manifest_lookup_p50_us: Option<f64>,
    pub recv_manifest_lookup_p95_us: Option<f64>,
    /// Estimated resident size of the manifest map.
    pub recv_manifest_memory_bytes: Option<u64>,
    /// One record per sampling interval, from the sender. Empty on a receiver
    /// report — receiver counters reach the sender on per-chunk headers.
    pub timeseries: Vec<TimeSample>,
    /// Whether throughput decayed, derived from `timeseries`. `null` when there
    /// were fewer than two windows to compare.
    pub throughput_decay: Option<ThroughputDecay>,
    /// The interval the samples were taken at. Doubles when the timeseries is
    /// downsampled, so a very long transfer stays bounded.
    pub sampled_every_secs: u64,
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
    /// The path this transfer took, as the UI names it: `LAN`, `P2P`,
    /// `Bluetooth`, `Relay` or `Tunnel`. `null` when this side did not choose
    /// (a receiver) or the sender had no manifest.
    pub transport: Option<String>,
    /// The manifest the control plane relayed with `START_TRANSFER`, verbatim —
    /// every medium the receiver advertised. `null` when there was none or this
    /// side never saw it.
    pub transport_manifest: Option<serde_json::Value>,
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
    /// The receiver-reported parts of `http`, and how many chunks reported any.
    recv_lock_wait_ns: AtomicU64,
    recv_part_write_ns: AtomicU64,
    recv_journal_ns: AtomicU64,
    recv_accept_ns: AtomicU64,
    recv_timed_chunks: AtomicU64,
    recv_lock_wait_latency: Mutex<LatencyHistogram>,
    recv_part_write_latency: Mutex<LatencyHistogram>,
    recv_journal_latency: Mutex<LatencyHistogram>,
    recv_accept_latency: Mutex<LatencyHistogram>,
    /// Receiver-side journal batching: one flush covers several chunks.
    journal_flushes: AtomicU64,
    journal_batch: Mutex<LatencyHistogram>,
    /// Receiver-side manifest writes, set once before the report is finalized.
    compaction_count: AtomicU64,
    compaction_total_ns: AtomicU64,
    compaction_under_lock_ns: AtomicU64,
    /// Receiver-side manifest size/lookups, set once before finalization.
    manifest_totals: Mutex<Option<ManifestTotals>>,
    /// Chunks dispatched by the scheduler and not yet settled.
    inflight_now: AtomicU32,
    /// The newest receiver counters, refreshed on every chunk ack.
    latest_recv: Mutex<ReceiverSnapshot>,
    /// The sampled time series and the interval it is being taken at.
    timeseries: Mutex<TimeseriesState>,
    timeseries_started: AtomicBool,
    /// How the transfer reached the peer, and the manifest it was chosen from.
    /// Set once at start; `null` when this side never knew.
    transport: Mutex<Option<String>>,
    transport_manifest: Mutex<Option<serde_json::Value>>,
    resources: Arc<ResourceSampler>,
    finalized: AtomicBool,
}

#[derive(Debug, Clone, Copy, Default)]
struct ManifestTotals {
    chunks: u32,
    memory_bytes: u64,
    lookup_p50_us: Option<f64>,
    lookup_p95_us: Option<f64>,
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
            recv_lock_wait_ns: AtomicU64::new(0),
            recv_part_write_ns: AtomicU64::new(0),
            recv_journal_ns: AtomicU64::new(0),
            recv_accept_ns: AtomicU64::new(0),
            recv_timed_chunks: AtomicU64::new(0),
            recv_lock_wait_latency: Mutex::new(LatencyHistogram::default()),
            recv_part_write_latency: Mutex::new(LatencyHistogram::default()),
            recv_journal_latency: Mutex::new(LatencyHistogram::default()),
            recv_accept_latency: Mutex::new(LatencyHistogram::default()),
            journal_flushes: AtomicU64::new(0),
            journal_batch: Mutex::new(LatencyHistogram::default()),
            compaction_count: AtomicU64::new(0),
            compaction_total_ns: AtomicU64::new(0),
            compaction_under_lock_ns: AtomicU64::new(0),
            manifest_totals: Mutex::new(None),
            inflight_now: AtomicU32::new(0),
            latest_recv: Mutex::new(ReceiverSnapshot::default()),
            timeseries: Mutex::new(TimeseriesState {
                samples: Vec::new(),
                interval_secs: SAMPLE_INTERVAL_SECS,
            }),
            timeseries_started: AtomicBool::new(false),
            files: Mutex::new(files),
            transport: Mutex::new(None),
            transport_manifest: Mutex::new(None),
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

    /// What one chunk's receiver reported about its durability step.
    ///
    /// `present` is false for a receiver that predates the timing headers; the
    /// chunk then counts towards the denominator of the coverage figure and
    /// nothing else. Zero durations are skipped by `record_phase`, so a 0 ms
    /// local contributes no sample rather than a fake one.
    pub fn chunk_receiver_phases(
        &self,
        lock_wait: std::time::Duration,
        part_write: std::time::Duration,
        journal: std::time::Duration,
        accept: std::time::Duration,
        present: bool,
    ) {
        if !present {
            return;
        }

        self.recv_timed_chunks.fetch_add(1, Ordering::Relaxed);

        record_phase(&self.recv_lock_wait_ns, &self.recv_lock_wait_latency, lock_wait);
        record_phase(&self.recv_part_write_ns, &self.recv_part_write_latency, part_write);
        record_phase(&self.recv_journal_ns, &self.recv_journal_latency, journal);
        record_phase(&self.recv_accept_ns, &self.recv_accept_latency, accept);
    }

    /// Count one dispatched chunk; the returned guard releases it.
    pub fn begin_chunk(self: &Arc<Self>) -> InFlightGuard {
        self.inflight_now.fetch_add(1, Ordering::Relaxed);

        InFlightGuard(Arc::clone(self))
    }

    /// Refresh the receiver counters from one acknowledgement.
    ///
    /// A field the receiver did not send keeps its previous value: an old
    /// receiver sends none, and the sample then carries `null`.
    pub fn observe_receiver(&self, snapshot: ReceiverSnapshot) {
        let mut latest = self.latest_recv.lock().expect("telemetry lock poisoned");

        if snapshot.inflight.is_some() {
            latest.inflight = snapshot.inflight;
        }

        if snapshot.journal_pending.is_some() {
            latest.journal_pending = snapshot.journal_pending;
        }

        if snapshot.part_files.is_some() {
            latest.part_files = snapshot.part_files;
        }

        if snapshot.incoming_bytes.is_some() {
            latest.incoming_bytes = snapshot.incoming_bytes;
        }

        if snapshot.compactions.is_some() {
            latest.compactions = snapshot.compactions;
        }

        if snapshot.journal_flushes.is_some() {
            latest.journal_flushes = snapshot.journal_flushes;
        }

        if snapshot.journal_entries.is_some() {
            latest.journal_entries = snapshot.journal_entries;
        }

        if snapshot.compaction_ms.is_some() {
            latest.compaction_ms = snapshot.compaction_ms;
        }

        if snapshot.compaction_under_lock_ms.is_some() {
            latest.compaction_under_lock_ms = snapshot.compaction_under_lock_ms;
        }

        if snapshot.manifest_chunks.is_some() {
            latest.manifest_chunks = snapshot.manifest_chunks;
        }

        if snapshot.manifest_lookup_p50_us.is_some() {
            latest.manifest_lookup_p50_us = snapshot.manifest_lookup_p50_us;
        }

        if snapshot.manifest_lookup_p95_us.is_some() {
            latest.manifest_lookup_p95_us = snapshot.manifest_lookup_p95_us;
        }

        if snapshot.manifest_memory_bytes.is_some() {
            latest.manifest_memory_bytes = snapshot.manifest_memory_bytes;
        }
    }

    /// The receiver's own manifest size and lookup cost, for its report. The
    /// sender gets the same numbers on the chunk headers instead.
    pub fn set_manifest_totals(
        &self,
        chunks: u32,
        memory_bytes: u64,
        lookups: LatencySummary,
    ) {
        *self
            .manifest_totals
            .lock()
            .expect("telemetry manifest lock poisoned") = Some(ManifestTotals {
            chunks,
            memory_bytes,
            lookup_p50_us: lookups.p50_ms.map(|ms| round(ms * 1000.0)),
            lookup_p95_us: lookups.p95_ms.map(|ms| round(ms * 1000.0)),
        });
    }

    /// The receiver's own manifest-write totals. The sender gets the same
    /// numbers on the chunk headers and this is never called for it.
    pub fn set_compaction_totals(&self, count: u64, total_ns: u64, under_lock_ns: u64) {
        self.compaction_count.store(count, Ordering::Relaxed);
        self.compaction_total_ns.store(total_ns, Ordering::Relaxed);
        self.compaction_under_lock_ns
            .store(under_lock_ns, Ordering::Relaxed);
    }

    /// The receiver's cumulative journal flushes and journaled entries, for the
    /// chunk acknowledgement headers.
    pub fn journal_totals(&self) -> (u64, u64) {
        let entries = self
            .journal_batch
            .lock()
            .expect("telemetry latency lock poisoned")
            .sum_ns;

        (self.journal_flushes.load(Ordering::Relaxed), entries)
    }

    /// Start the per-interval sampler. Once per transfer.
    ///
    /// Only the sender samples: the receiver is a different process, and its
    /// counters reach the sender on the per-chunk acknowledgement headers.
    pub fn start_timeseries(self: &Arc<Self>) {
        if !matches!(self.role, Role::Sender) {
            return;
        }

        if self.timeseries_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };

        let telemetry = Arc::clone(self);

        handle.spawn(async move {
            let mut last_bytes = 0u64;
            let mut last_latency_count = 0u64;
            let mut last_latency_ns = 0u64;
            let mut last_compactions: Option<u64> = None;
            let mut last_journal_flushes: Option<u64> = None;
            let mut last_recv_lock_wait_ns = 0u64;
            let mut last_recv_journal_ns = 0u64;

            loop {
                let interval_secs = {
                    telemetry
                        .timeseries
                        .lock()
                        .expect("telemetry lock poisoned")
                        .interval_secs
                };

                tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

                if telemetry.finalized.load(Ordering::Relaxed) {
                    break;
                }

                let t_secs = telemetry.now_ns() as f64 / 1e9;

                // One bad sample is reported in its own record and never fails
                // the transfer.
                let collected = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    telemetry.collect_sample(
                        t_secs,
                        interval_secs,
                        &mut last_bytes,
                        &mut last_latency_count,
                        &mut last_latency_ns,
                        &mut last_compactions,
                        &mut last_journal_flushes,
                        &mut last_recv_lock_wait_ns,
                        &mut last_recv_journal_ns,
                    )
                }));

                match collected {
                    Ok(sample) => telemetry.push_sample(sample),

                    Err(_) => telemetry.push_sample(TimeSample {
                        t_secs,
                        bytes_acked: 0,
                        chunks_acked: 0,
                        window_mib_s: 0.0,
                        inflight_now: 0,
                        sender_rss_bytes: 0,
                        sender_open_fds: None,
                        recv_rss_bytes: None,
                        recv_inflight_map_len: None,
                        recv_journal_pending: None,
                        recv_journal_flushes_this_window: None,
                        recv_manifest_chunk_count: None,
                        recv_part_files: None,
                        recv_incoming_dir_bytes: None,
                        compactions_this_window: None,
                        avg_chunk_latency_ms_this_window: 0.0,
                        error: Some("sample collection panicked".into()),
                    }),
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_sample(
        &self,
        t_secs: f64,
        interval_secs: u64,
        last_bytes: &mut u64,
        last_latency_count: &mut u64,
        last_latency_ns: &mut u64,
        last_compactions: &mut Option<u64>,
        last_journal_flushes: &mut Option<u64>,
        last_recv_lock_wait_ns: &mut u64,
        last_recv_journal_ns: &mut u64,
    ) -> TimeSample {
        let bytes = self.plaintext_bytes.load(Ordering::Relaxed);

        let (latency_count, latency_sum) = {
            let histogram = self.latency.lock().expect("telemetry latency lock poisoned");

            (histogram.count, histogram.sum_ns)
        };

        let delta_bytes = bytes.saturating_sub(*last_bytes);
        let delta_count = latency_count.saturating_sub(*last_latency_count);
        let delta_ns = latency_sum.saturating_sub(*last_latency_ns);

        let avg_chunk_latency_ms = if delta_count > 0 {
            delta_ns as f64 / delta_count as f64 / 1e6
        } else {
            0.0
        };

        let snapshot = self
            .latest_recv
            .lock()
            .expect("telemetry lock poisoned")
            .clone();

        let compactions_this_window = snapshot.compactions.map(|total| {
            let delta = last_compactions
                .map(|previous| total.saturating_sub(previous))
                .unwrap_or(total);

            *last_compactions = Some(total);

            u32::try_from(delta).unwrap_or(u32::MAX)
        });

        let journal_flushes_this_window = snapshot.journal_flushes.map(|total| {
            let delta = last_journal_flushes
                .map(|previous| total.saturating_sub(previous))
                .unwrap_or(total);

            *last_journal_flushes = Some(total);

            u32::try_from(delta).unwrap_or(u32::MAX)
        });

        /*
         * The receiver's per-chunk durability totals are cumulative; the log
         * carries the average over this window so a killed transfer still
         * leaves the decay curve behind.
         */
        let recv_lock_wait_ms_this_window = window_average_ms(
            self.recv_lock_wait_ns.load(Ordering::Relaxed),
            last_recv_lock_wait_ns,
            delta_count,
        );

        let recv_journal_ms_this_window = window_average_ms(
            self.recv_journal_ns.load(Ordering::Relaxed),
            last_recv_journal_ns,
            delta_count,
        );

        *last_bytes = bytes;
        *last_latency_count = latency_count;
        *last_latency_ns = latency_sum;

        tracing::info!(
            target: "transfer::series",
            t_secs,
            window_mib_s = delta_bytes as f64 / interval_secs as f64 / (1024.0 * 1024.0),
            recv_lock_wait_ms = recv_lock_wait_ms_this_window,
            recv_journal_ms = recv_journal_ms_this_window,
            recv_part_files = ?snapshot.part_files,
            sender_open_fds = ?open_fd_count(),
            "sample"
        );

        TimeSample {
            t_secs,
            bytes_acked: bytes,
            chunks_acked: latency_count,
            window_mib_s: delta_bytes as f64 / interval_secs as f64 / (1024.0 * 1024.0),
            inflight_now: self.inflight_now.load(Ordering::Relaxed),
            sender_rss_bytes: self.resources.current_rss(),
            sender_open_fds: open_fd_count(),
            recv_rss_bytes: None,
            recv_inflight_map_len: snapshot.inflight,
            recv_journal_pending: snapshot.journal_pending,
            recv_journal_flushes_this_window: journal_flushes_this_window,
            recv_manifest_chunk_count: snapshot.manifest_chunks,
            recv_part_files: snapshot.part_files,
            recv_incoming_dir_bytes: snapshot.incoming_bytes,
            compactions_this_window,
            avg_chunk_latency_ms_this_window: avg_chunk_latency_ms,
            error: None,
        }
    }

    fn push_sample(&self, sample: TimeSample) {
        let mut series = self.timeseries.lock().expect("telemetry lock poisoned");

        series.samples.push(sample);

        if series.samples.len() > MAX_TIMESERIES_SAMPLES {
            let mut kept = Vec::with_capacity(MAX_TIMESERIES_SAMPLES / 2 + 1);

            for (index, sample) in series.samples.drain(..).enumerate() {
                if index % 2 == 0 {
                    kept.push(sample);
                }
            }

            series.samples = kept;
            series.interval_secs = series.interval_secs.saturating_mul(2);
        }
    }

    /// One journal flush, covering `entries` chunks.
    pub fn record_journal_flush(&self, entries: u64) {
        self.journal_flushes.fetch_add(1, Ordering::Relaxed);

        self.journal_batch
            .lock()
            .expect("telemetry latency lock poisoned")
            .record(entries);
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

        let recv_lock_wait = self.recv_lock_wait_ns.load(Ordering::Relaxed);
        let recv_part_write = self.recv_part_write_ns.load(Ordering::Relaxed);
        let recv_journal = self.recv_journal_ns.load(Ordering::Relaxed);
        let recv_accept = self.recv_accept_ns.load(Ordering::Relaxed);

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
            recv_lock_wait: PhaseShare {
                total_seconds: seconds(recv_lock_wait),
                share_percent: share(recv_lock_wait),
                latency: self
                    .recv_lock_wait_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            recv_part_write: PhaseShare {
                total_seconds: seconds(recv_part_write),
                share_percent: share(recv_part_write),
                latency: self
                    .recv_part_write_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            recv_journal: PhaseShare {
                total_seconds: seconds(recv_journal),
                share_percent: share(recv_journal),
                latency: self
                    .recv_journal_latency
                    .lock()
                    .expect("telemetry latency lock poisoned")
                    .summary(),
            },
            recv_accept: PhaseShare {
                total_seconds: seconds(recv_accept),
                share_percent: share(recv_accept),
                latency: self
                    .recv_accept_latency
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

    /// Records how this transfer reached the peer.
    ///
    /// Set once, at start, from what the sender chose and the manifest it chose
    /// from. The receiver never learns either, so its report leaves them null.
    pub fn set_transport(
        &self,
        transport: Option<String>,
        manifest: Option<serde_json::Value>,
    ) {
        *self
            .transport
            .lock()
            .expect("telemetry transport lock poisoned") = transport;
        *self
            .transport_manifest
            .lock()
            .expect("telemetry transport manifest lock poisoned") = manifest;
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

            let precompressed = descriptor
                .relative_path
                .as_deref()
                .map_or(false, |path| {
                    crate::transfer::compression::is_precompressed(Path::new(path))
                });

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
                compression_enabled: !precompressed,
                compression_skipped_reason: precompressed.then_some("precompressed_ext"),
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

        /*
         * How much of the sender's own `http` time the receiver accounted for.
         * The denominator is the acknowledged-chunk count, so a receiver that
         * sends no timing headers reads 0% coverage rather than being silently
         * assumed to have sent zeros.
         */
        let acknowledged = self
            .latency
            .lock()
            .expect("telemetry latency lock poisoned")
            .count;

        let recv_timed = self.recv_timed_chunks.load(Ordering::Relaxed);

        let recv_timings_coverage_percent =
            (acknowledged > 0).then(|| round(recv_timed as f64 / acknowledged as f64 * 100.0));

        let http_nanos = self.http_ns.load(Ordering::Relaxed);

        let recv_nanos = self
            .recv_lock_wait_ns
            .load(Ordering::Relaxed)
            .saturating_add(self.recv_part_write_ns.load(Ordering::Relaxed))
            .saturating_add(self.recv_journal_ns.load(Ordering::Relaxed))
            .saturating_add(self.recv_accept_ns.load(Ordering::Relaxed));

        let recv_durability_share_percent = match recv_timings_coverage_percent {
            Some(coverage) if coverage >= 10.0 && http_nanos > 0 => {
                Some(round(recv_nanos as f64 / http_nanos as f64 * 100.0))
            }

            _ => None,
        };

        let (timeseries, sampled_every_secs) = {
            let series = self.timeseries.lock().expect("telemetry lock poisoned");

            (series.samples.clone(), series.interval_secs)
        };

        /*
         * A sender learns the receiver's journal batching from its per-chunk
         * headers; a receiver reports its own. When no header ever arrived, the
         * local counters answer — which is `0` on a sender.
         */
        let (recv_journal_flush_count, recv_journal_entries_per_flush) = {
            let latest = self
                .latest_recv
                .lock()
                .expect("telemetry lock poisoned")
                .clone();

            match (latest.journal_flushes, latest.journal_entries) {
                (Some(flushes), Some(entries)) if flushes > 0 => (
                    flushes,
                    Some(CountSummary {
                        count: flushes,
                        mean: Some(round(entries as f64 / flushes as f64)),
                        p50: None,
                        p95: None,
                        max: None,
                        resolution: "receiver cumulative: journaled entries per flush",
                    }),
                ),

                _ => {
                    let summary = self
                        .journal_batch
                        .lock()
                        .expect("telemetry latency lock poisoned")
                        .raw_summary();

                    (
                        self.journal_flushes.load(Ordering::Relaxed),
                        (summary.count > 0).then_some(summary),
                    )
                }
            }
        };

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

        let transport = self
            .transport
            .lock()
            .expect("telemetry transport lock poisoned")
            .clone();

        let transport_manifest = self
            .transport_manifest
            .lock()
            .expect("telemetry transport manifest lock poisoned")
            .clone();

        if transport.is_none() {
            unavailable.push("transport (not chosen, or not observed on this side)".into());
        }

        if transport_manifest.is_none() {
            unavailable.push("transport_manifest (no manifest was relayed)".into());
        }

        let (recv_compaction_count, recv_compaction_total_seconds, recv_compaction_under_lock_ms) = {
            let latest = self
                .latest_recv
                .lock()
                .expect("telemetry lock poisoned")
                .clone();

            if let Some(count) = latest.compactions {
                (
                    count,
                    latest.compaction_ms.map(|ms| round(ms as f64 / 1000.0)),
                    latest.compaction_under_lock_ms.map(|ms| ms as f64),
                )
            } else if matches!(self.role, Role::Receiver) {
                let total_ns = self.compaction_total_ns.load(Ordering::Relaxed);
                let under_ns = self.compaction_under_lock_ns.load(Ordering::Relaxed);

                (
                    self.compaction_count.load(Ordering::Relaxed),
                    (total_ns > 0).then_some(round(total_ns as f64 / 1e9)),
                    Some(round(under_ns as f64 / 1e6)),
                )
            } else {
                (0, None, None)
            }
        };

        let (
            recv_manifest_chunk_count,
            recv_manifest_lookup_p50_us,
            recv_manifest_lookup_p95_us,
            recv_manifest_memory_bytes,
        ) = {
            let latest = self
                .latest_recv
                .lock()
                .expect("telemetry lock poisoned")
                .clone();

            if let Some(chunks) = latest.manifest_chunks {
                (
                    Some(chunks),
                    latest.manifest_lookup_p50_us.map(|value| value as f64),
                    latest.manifest_lookup_p95_us.map(|value| value as f64),
                    latest.manifest_memory_bytes,
                )
            } else if let Some(own) = *self
                .manifest_totals
                .lock()
                .expect("telemetry manifest lock poisoned")
            {
                (
                    Some(own.chunks),
                    own.lookup_p50_us,
                    own.lookup_p95_us,
                    Some(own.memory_bytes),
                )
            } else {
                (None, None, None, None)
            }
        };

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
            recv_durability_share_percent,
            recv_timings_coverage_percent,
            recv_journal_flush_count,
            recv_journal_entries_per_flush,
            recv_compaction_count,
            recv_compaction_total_seconds,
            recv_compaction_under_lock_ms,
            recv_manifest_chunk_count,
            recv_manifest_lookup_p50_us,
            recv_manifest_lookup_p95_us,
            recv_manifest_memory_bytes,
            timeseries: timeseries.clone(),
            throughput_decay: throughput_decay(&timeseries),
            sampled_every_secs,
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
            transport,
            transport_manifest,
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

/// How many handles/fds this process has open, where the platform can say.
///
/// A syscall on Windows and a directory read on Linux; `None` elsewhere rather
/// than a guess. Called once per sampling interval, not per chunk.
fn open_fd_count() -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn GetProcessHandleCount(process: isize, count: *mut u32) -> i32;
        }

        let mut count = 0u32;

        // SAFETY: both are plain Win32 calls; `count` is a valid out pointer
        // and the returned handle is the current process pseudo-handle.
        let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };

        (ok != 0).then_some(count)
    }

    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/fd")
            .ok()
            .map(|entries| entries.filter_map(Result::ok).count() as u32)
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        None
    }
}

/// `first / last` throughput ratio, and the first sample below 70% of the
/// first quartile.
fn throughput_decay(samples: &[TimeSample]) -> Option<ThroughputDecay> {
    if samples.len() < 2 {
        return None;
    }

    let quartile = (samples.len() / 4).max(1);

    let mean = |slice: &[TimeSample]| -> f64 {
        slice.iter().map(|sample| sample.window_mib_s).sum::<f64>() / slice.len() as f64
    };

    let first = mean(&samples[..quartile]);
    let last = mean(&samples[samples.len() - quartile..]);

    // Floored so a total stall is still a finite number in the JSON.
    let decay_ratio = first / last.max(0.001);

    let slowdown_onset_secs = (first > 0.0)
        .then(|| {
            samples
                .iter()
                .find(|sample| sample.window_mib_s < 0.7 * first)
                .map(|sample| round(sample.t_secs))
        })
        .flatten();

    Some(ThroughputDecay {
        first_quartile_mib_s: round(first),
        last_quartile_mib_s: round(last),
        decay_ratio: round(decay_ratio),
        slowdown_onset_secs,
    })
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

pub(crate) fn status_name(status: TransferStatus) -> &'static str {
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
        "- role: {}\n- schema: {}\n- duration: {}\n- bytes: {} of {} acknowledged ({} remaining)\n- chunks: {} of {}\n- files: {} completed, {} unknown, {} total\n- compression: {}\n- integrity digests: {}\n- recovered after restart: {}\n",
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

    out.push_str(&format!(
        "- transport: {}\n- transport manifest: {}\n\n",
        report.transport.as_deref().unwrap_or("n/a"),
        report
            .transport_manifest
            .as_ref()
            .map(|manifest| manifest.to_string())
            .unwrap_or_else(|| "n/a".into()),
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
    async fn a_sender_report_records_the_transport_and_the_manifest_it_chose_from() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Sender);

        telemetry.set_transport(
            Some("LAN".into()),
            Some(serde_json::json!({
                "manifest_version": 1,
                "capabilities": { "lan": true, "p2p": false, "ble": false },
                "lan": { "endpoints": [{ "ip": "192.168.1.5", "port": 7878 }] }
            })),
        );

        let paths = telemetry
            .finalize(
                &reports,
                finish("t-1", TransferStatus::Completed),
                false,
                false,
            )
            .await
            .expect("the report is written");

        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&paths.json).expect("reading")).expect("json");

        assert_eq!(json["transport"], "LAN");
        assert_eq!(json["transport_manifest"]["capabilities"]["lan"], true);
        assert_eq!(
            json["transport_manifest"]["lan"]["endpoints"][0]["ip"],
            "192.168.1.5",
            "the manifest is recorded verbatim, not summarised"
        );

        let markdown = std::fs::read_to_string(&paths.markdown).expect("reading");

        assert!(markdown.contains("- transport: LAN"));
        assert!(markdown.contains("192.168.1.5"));
    }

    #[tokio::test]
    async fn a_side_that_never_saw_a_manifest_reports_them_null_and_unavailable() {
        let scratch = TempDir::new();
        let reports = scratch.path().join("reports");
        let telemetry = Telemetry::new(Role::Receiver);

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

        assert!(json["transport"].is_null());
        assert!(json["transport_manifest"].is_null());

        let unavailable = json["unavailable"].as_array().expect("unavailable");

        assert!(unavailable
            .iter()
            .any(|item| item.as_str().unwrap_or_default().starts_with("transport ")));
        assert!(unavailable.iter().any(|item| item
            .as_str()
            .unwrap_or_default()
            .starts_with("transport_manifest")));
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
    fn receiver_durability_share_is_null_when_no_chunk_reported_timings() {
        let telemetry = Telemetry::new(Role::Sender);

        for _ in 0..10 {
            telemetry.chunk_finished("f-1", std::time::Duration::from_millis(100), 0);
            telemetry.chunk_phases(
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                std::time::Duration::from_millis(100),
            );
        }

        let report = telemetry.build(finish("t-1", TransferStatus::Completed), false, false);

        assert_eq!(report.recv_timings_coverage_percent, Some(0.0));
        assert_eq!(
            report.recv_durability_share_percent, None,
            "0% coverage must not be reported as a 0% share"
        );
    }

    #[test]
    fn receiver_durability_share_attributes_the_reported_part_of_http() {
        let telemetry = Telemetry::new(Role::Sender);

        for _ in 0..10 {
            telemetry.chunk_finished("f-1", std::time::Duration::from_millis(100), 0);
            telemetry.chunk_phases(
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                std::time::Duration::ZERO,
                std::time::Duration::from_millis(100),
            );
            telemetry.chunk_receiver_phases(
                std::time::Duration::from_millis(10),
                std::time::Duration::from_millis(20),
                std::time::Duration::from_millis(5),
                std::time::Duration::from_millis(40),
                true,
            );
        }

        let report = telemetry.build(finish("t-1", TransferStatus::Completed), false, false);

        assert_eq!(report.recv_timings_coverage_percent, Some(100.0));
        // (10 + 20 + 5 + 40) ms per chunk over 100 ms of http: 75%.
        assert_eq!(report.recv_durability_share_percent, Some(75.0));
    }

    fn sample(t_secs: u64, window_mib_s: f64) -> TimeSample {
        TimeSample {
            t_secs: t_secs as f64,
            bytes_acked: 0,
            chunks_acked: 0,
            window_mib_s,
            inflight_now: 0,
            sender_rss_bytes: 0,
            sender_open_fds: None,
            recv_rss_bytes: None,
            recv_inflight_map_len: None,
            recv_journal_pending: None,
            recv_journal_flushes_this_window: None,
            recv_manifest_chunk_count: None,
            recv_part_files: None,
            recv_incoming_dir_bytes: None,
            compactions_this_window: None,
            avg_chunk_latency_ms_this_window: 0.0,
            error: None,
        }
    }

    #[test]
    fn throughput_decay_finds_the_slowdown() {
        let samples: Vec<TimeSample> = [100.0, 100.0, 100.0, 100.0, 90.0, 60.0, 20.0, 10.0]
            .iter()
            .enumerate()
            .map(|(index, window)| sample(index as u64, *window))
            .collect();

        let decay = throughput_decay(&samples).expect("eight windows");

        assert_eq!(decay.first_quartile_mib_s, 100.0);
        assert_eq!(decay.last_quartile_mib_s, 15.0);
        assert_eq!(decay.decay_ratio, 6.667);
        assert_eq!(
            decay.slowdown_onset_secs,
            Some(5.0),
            "the first window below 70% of the first quartile"
        );
    }

    #[test]
    fn a_flat_transfer_has_no_decay_and_no_onset() {
        let samples: Vec<TimeSample> = (0..8).map(|index| sample(index, 100.0)).collect();

        let decay = throughput_decay(&samples).expect("eight windows");

        assert_eq!(decay.decay_ratio, 1.0);
        assert_eq!(decay.slowdown_onset_secs, None);
    }

    #[test]
    fn the_timeseries_is_capped_and_downsampled() {
        let telemetry = Telemetry::new(Role::Sender);

        for second in 0..800u64 {
            telemetry.push_sample(sample(second, 1.0));
        }

        let series = telemetry.timeseries.lock().expect("series");

        assert!(
            series.samples.len() <= MAX_TIMESERIES_SAMPLES,
            "the series must stay bounded"
        );
        assert_eq!(series.interval_secs, SAMPLE_INTERVAL_SECS * 2);
    }

    #[test]
    fn receiver_compaction_totals_reach_the_report() {
        let telemetry = Telemetry::new(Role::Receiver);

        telemetry.set_compaction_totals(3, 1_500_000_000, 0);

        let report = telemetry.build(finish("t-1", TransferStatus::Completed), false, false);

        assert_eq!(report.recv_compaction_count, 3);
        assert_eq!(report.recv_compaction_total_seconds, Some(1.5));
        assert_eq!(report.recv_compaction_under_lock_ms, Some(0.0));
    }

    #[test]
    fn journal_flushes_are_counted_and_sized() {
        let telemetry = Telemetry::new(Role::Receiver);

        telemetry.record_journal_flush(32);
        telemetry.record_journal_flush(8);

        let report = telemetry.build(finish("t-1", TransferStatus::Completed), false, false);

        assert_eq!(report.recv_journal_flush_count, 2);

        let stats = report
            .recv_journal_entries_per_flush
            .expect("two flushes were recorded");

        assert_eq!(stats.count, 2);
        assert_eq!(stats.mean, Some(20.0));
        assert_eq!(stats.max, Some(32));
        assert!(stats.p50.unwrap() <= 32);
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
