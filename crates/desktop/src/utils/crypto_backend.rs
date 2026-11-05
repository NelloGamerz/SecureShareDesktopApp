//! One-time startup report of the AES-GCM hardware backend.
//!
//! A software fallback is silent: `aes` 0.8 and `polyval` 0.6 pick their
//! backends at compile time, so nothing at runtime can tell you that every
//! chunk is being encrypted ~8x slower than the CPU allows. This logs it once
//! so a transfer report showing a large `encrypt` bucket can be read against
//! what the build actually compiled.
//!
//! * `x86_64` — `aes` and `polyval` both dispatch at runtime through
//!   `cpufeatures`, so the CPU flags are the whole answer.
//! * `aarch64` — they only use the ARMv8 crypto extensions when the build was
//!   given `--cfg aes_armv8 --cfg polyval_armv8`; without them the backend is
//!   software regardless of the CPU. Both facts are reported.

pub fn log_aes_backend() {
    #[cfg(target_arch = "x86_64")]
    tracing::info!(
        target: "crypto",
        arch = std::env::consts::ARCH,
        backend = "runtime-dispatch",
        aes = std::arch::is_x86_feature_detected!("aes"),
        pclmulqdq = std::arch::is_x86_feature_detected!("pclmulqdq"),
        "AES-GCM hardware support"
    );

    #[cfg(target_arch = "aarch64")]
    tracing::info!(
        target: "crypto",
        arch = std::env::consts::ARCH,
        backend = if cfg!(aes_armv8) && cfg!(polyval_armv8) {
            "armv8"
        } else {
            "software-fallback"
        },
        aes = std::arch::is_aarch64_feature_detected!("aes"),
        pmull = std::arch::is_aarch64_feature_detected!("pmull"),
        "AES-GCM hardware support"
    );

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    tracing::info!(
        target: "crypto",
        arch = std::env::consts::ARCH,
        backend = "software-fallback",
        "AES-GCM hardware support"
    );
}
