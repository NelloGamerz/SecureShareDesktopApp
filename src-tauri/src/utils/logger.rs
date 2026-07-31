pub struct Logger;

impl Logger {
    pub fn init() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("server_frontend=info")
            .with_target(false)
            .try_init();
    }
}
