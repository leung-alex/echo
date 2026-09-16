//! Fixed-size, ephemeral target-thread observation. No key or edit commands.
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAGIC: u64 = 0x4543484f494d4503;
pub const MAX_UNITS: usize = 2048;
pub const MESSAGE: &str = "Echo.CompositionObservation.v3";
pub const PREFIX: &str = "Local\\Echo.CompositionObservation.";

#[repr(C)]
pub struct Channel {
    pub magic: u64,
    pub target_pid: u32,
    pub target_thread: u32,
    pub target_window: u64,
    pub target_started: u64,
    pub tsf_only: u32,
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
    pub mode: u32,
    pub composition: u32,
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
            mode: 0,
            composition: 0,
            units: 0,
            text: [0; MAX_UNITS],
        }
    }
}

// Read-only status requests never populate text. Values are deliberately separate
// from legacy composition status so a missing value cannot become English/idle.
pub const STATE_ONLY: u32 = 2;
pub const STATE_REPLY: u32 = 5;
#[allow(dead_code)] // Also compiled into the standalone observer DLL.
pub fn mode(language: u16, bits: Option<(bool, u32)>) -> u32 {
    match (language & 0x3ff, bits) {
        (0x04, Some((true, conversion))) if conversion & 1 != 0 => 1,
        (0x04, Some(_)) => 2,
        (0x09, _) => 2,
        _ => 0,
    }
}
#[allow(dead_code)] // Also compiled into the standalone observer DLL.
pub fn agree(tsf: Option<(bool, u32)>, imm: Option<(bool, u32)>) -> Option<(bool, u32)> {
    // Full-width, punctuation and Roman flags do not change the 中/EN label.
    let normalize = |(open, conversion): (bool, u32)| (open, if open { conversion & 1 } else { 0 });
    let (tsf, imm) = (tsf.map(normalize), imm.map(normalize));
    match (tsf, imm) {
        (Some(a), Some(b)) if a != b => None,
        (Some(a), _) | (_, Some(a)) => Some(a),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversion_mode_requires_language_and_explicit_evidence() {
        assert_eq!(mode(0x804, Some((true, 1))), 1);
        assert_eq!(mode(0x804, Some((true, 0))), 2);
        assert_eq!(mode(0x804, Some((false, 1))), 2);
        assert_eq!(mode(0x804, None), 0);
        assert_eq!(mode(0x409, None), 2);
        assert_eq!(mode(0x411, Some((true, 1))), 0);
        assert_eq!(mode(0x412, Some((true, 0))), 0);
        assert_eq!(mode(0, Some((true, 0))), 0);
    }
    #[test]
    fn conflicting_or_missing_sources_never_guess_english() {
        assert_eq!(agree(None, None), None);
        assert_eq!(agree(Some((true, 1)), Some((true, 0))), None);
        assert_eq!(agree(Some((true, 1)), Some((false, 1))), None);
        assert_eq!(agree(Some((true, 1)), None), Some((true, 1)));
        assert_eq!(agree(None, Some((true, 0))), Some((true, 0)));
        assert_eq!(agree(Some((true, 0x401)), Some((true, 1))), Some((true, 1)));
    }
}
impl Channel {
    pub fn new(pid: u32, thread: u32, window: u64, started: u64, tsf_only: bool) -> Self {
        Self {
            magic: MAGIC,
            target_pid: pid,
            target_thread: thread,
            target_window: window,
            target_started: started,
            tsf_only: u32::from(tsf_only),
            request: AtomicU64::new(0),
            response: AtomicU64::new(0),
            sample: Sample::unknown(),
        }
    }
    pub fn next_request(&self) -> u64 {
        self.request.fetch_add(1, Ordering::AcqRel) + 1
    }
}
