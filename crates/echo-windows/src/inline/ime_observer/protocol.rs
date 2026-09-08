//! Fixed-size, ephemeral target-thread observation. No key or edit commands.
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAGIC: u64 = 0x4543484f494d4501;
pub const MAX_UNITS: usize = 2048;
pub const MESSAGE: &str = "Echo.CompositionObservation.v1";
pub const PREFIX: &str = "Local\\Echo.CompositionObservation.";

#[repr(C)]
pub struct Channel {
    pub magic: u64,
    pub target_pid: u32,
    pub target_thread: u32,
    pub target_window: u64,
    pub target_started: u64,
    pub request: AtomicU64,
    pub response: AtomicU64,
    pub sample: Sample,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Sample {
    pub pid: u32,
    pub thread: u32,
    pub window: u64,
    pub status: u32,
    pub units: u32,
    pub text: [u16; MAX_UNITS],
}
impl Sample {
    pub const fn unknown() -> Self {
        Self {
            pid: 0,
            thread: 0,
            window: 0,
            status: 0,
            units: 0,
            text: [0; MAX_UNITS],
        }
    }
}
impl Channel {
    pub fn new(pid: u32, thread: u32, window: u64, started: u64) -> Self {
        Self {
            magic: MAGIC,
            target_pid: pid,
            target_thread: thread,
            target_window: window,
            target_started: started,
            request: AtomicU64::new(0),
            response: AtomicU64::new(0),
            sample: Sample::unknown(),
        }
    }
    pub fn next_request(&self) -> u64 {
        self.request.fetch_add(1, Ordering::AcqRel) + 1
    }
}
