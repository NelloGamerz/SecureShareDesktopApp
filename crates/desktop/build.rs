fn main() {
    // Set by `.cargo/config.toml` for aarch64; read by `aes`/`polyval` and
    // reported by `utils::crypto_backend`.
    println!("cargo::rustc-check-cfg=cfg(aes_armv8)");
    println!("cargo::rustc-check-cfg=cfg(polyval_armv8)");

    tauri_build::build()
}
