use std::process::Child;
use std::sync::Mutex;

pub struct Cloudflared {
    pub process: Mutex<Option<Child>>,
}

impl Cloudflared {
    pub fn new() -> Self {
        Self {
            process: Mutex::new(None),
        }
    }
}
