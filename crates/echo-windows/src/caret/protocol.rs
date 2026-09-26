//! `Echo.CaretObservation.v1` fixed-size content-free mailbox.
//!
//! This module is intentionally std-only: the same source is compiled into
//! the injected observer with standalone rustc. It carries identity, status,
//! geometry and HRESULT metadata only; it never carries text or COM pointers.

use std::mem::size_of;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub const MAGIC: u64 = u64::from_le_bytes(*b"ECHOCAR1");
pub const VERSION: u32 = 1;
pub const MAX_BYTES: usize = 4096;
pub const MAX_SNAPSHOT_RETRIES: usize = 3;

pub const REQUEST_PROBE: u32 = 1;
pub const REQUEST_CLOSE: u32 = 2;

pub const RESPONSE_EMPTY: u32 = 0;
pub const RESPONSE_PENDING: u32 = 1;
pub const RESPONSE_READY: u32 = 2;
pub const RESPONSE_UNAVAILABLE: u32 = 3;
pub const RESPONSE_CLOSED: u32 = 4;

/// Response word 26 carries the badge DPI only when the producer can prove a
/// monitor-effective value in the host coordinate space.  The TSF observer
/// deliberately publishes zero: words 18..21 are physical screen pixels and
/// Echo resolves the containing monitor/work-area/DPI on its own
/// per-monitor-aware thread.  Zero is therefore an explicit "host resolved"
/// marker, never a 96-DPI default.
pub const GEOMETRY_DPI_HOST_RESOLVED: u32 = 0;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub magic: u64,
    pub version: u32,
    pub byte_size: u32,
    pub nonce: [u32; 4],
    pub echo_pid: u32,
    pub target_pid: u32,
    pub input_thread: u32,
    pub root_pid: u32,
    pub echo_started: u64,
    pub target_started: u64,
    pub root_started: u64,
    pub root_hwnd: u64,
    pub input_hwnd: u64,
    pub focus_generation: u64,
    pub dll_digest: [u8; 32],
}

impl Header {
    pub fn for_mailbox(mut self) -> Self {
        self.byte_size = size_of::<Mailbox>() as u32;
        self
    }
}

#[repr(C)]
pub struct AtomicRecord<const N: usize> {
    revision: AtomicU32,
    words: [AtomicU32; N],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishError {
    Rollover,
    WriterBusy,
}

impl<const N: usize> AtomicRecord<N> {
    pub fn new() -> Self {
        Self {
            revision: AtomicU32::new(0),
            words: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }

    /// The designated single writer publishes a complete payload without any
    /// COM, Win32 send, allocation, or external callback.
    pub fn publish(&self, words: &[u32; N]) -> Result<(), PublishError> {
        let old = self.revision.load(Ordering::Acquire);
        if old & 1 != 0 {
            return Err(PublishError::WriterBusy);
        }
        let end = old.checked_add(2).ok_or(PublishError::Rollover)?;
        self.revision.store(old + 1, Ordering::Release);
        for (cell, value) in self.words.iter().zip(words.iter()) {
            cell.store(*value, Ordering::Relaxed);
        }
        self.revision.store(end, Ordering::Release);
        Ok(())
    }

    /// Snapshot at most three times. A permanently busy or torn publication
    /// becomes an unavailable observation instead of an unbounded spin.
    pub fn snapshot(&self) -> Option<[u32; N]> {
        for _ in 0..MAX_SNAPSHOT_RETRIES {
            let before = self.revision.load(Ordering::Acquire);
            if before & 1 != 0 {
                continue;
            }
            let values = std::array::from_fn(|i| self.words[i].load(Ordering::Relaxed));
            let after = self.revision.load(Ordering::Acquire);
            if before == after && after & 1 == 0 {
                return Some(values);
            }
        }
        None
    }

    #[cfg(test)]
    fn set_revision_for_test(&self, value: u32) {
        self.revision.store(value, Ordering::Relaxed);
    }
}

impl<const N: usize> Default for AtomicRecord<N> {
    fn default() -> Self {
        Self::new()
    }
}

#[repr(C, align(8))]
pub struct Mailbox {
    pub header: Header,
    pub request: AtomicRecord<16>,
    pub response: AtomicRecord<64>,
    pub heartbeat_tick: AtomicU64,
    pub closed: AtomicU32,
    pub scheduler_hwnd: AtomicU64,
}

impl Mailbox {
    pub fn new(header: Header) -> Self {
        Self {
            header: header.for_mailbox(),
            request: AtomicRecord::new(),
            response: AtomicRecord::new(),
            heartbeat_tick: AtomicU64::new(0),
            closed: AtomicU32::new(0),
            scheduler_hwnd: AtomicU64::new(0),
        }
    }
}

pub fn header_shape_valid(header: &Header) -> bool {
    header.magic == MAGIC
        && header.version == VERSION
        && header.byte_size as usize == size_of::<Mailbox>()
        && header.byte_size as usize <= MAX_BYTES
        && header.nonce.iter().any(|word| *word != 0)
        && header.echo_pid != 0
        && header.target_pid != 0
        && header.root_pid != 0
        && header.input_thread != 0
        && header.echo_started != 0
        && header.target_started != 0
        && header.root_started != 0
        && header.root_hwnd != 0
        && header.input_hwnd != 0
        && header.focus_generation != 0
        && header.dll_digest.iter().any(|byte| *byte != 0)
}

/// Header fields are immutable for a mailbox lifetime. This comparison is
/// separate from live HWND/process/session validation performed by each side.
pub fn header_identity_matches(expected: &Header, actual: &Header) -> bool {
    expected.magic == actual.magic
        && expected.version == actual.version
        && expected.byte_size == actual.byte_size
        && expected.nonce == actual.nonce
        && expected.echo_pid == actual.echo_pid
        && expected.target_pid == actual.target_pid
        && expected.input_thread == actual.input_thread
        && expected.root_pid == actual.root_pid
        && expected.echo_started == actual.echo_started
        && expected.target_started == actual.target_started
        && expected.root_started == actual.root_started
        && expected.root_hwnd == actual.root_hwnd
        && expected.input_hwnd == actual.input_hwnd
        && expected.focus_generation == actual.focus_generation
        && expected.dll_digest == actual.dll_digest
}

pub fn response_matches_request(words: &[u32; 64], request_sequence: u32) -> bool {
    words[1] == request_sequence && words[0] != RESPONSE_EMPTY
}

pub const fn i32_bits(value: i32) -> u32 {
    value as u32
}

pub const fn bits_i32(value: u32) -> i32 {
    value as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::align_of;

    fn header() -> Header {
        Header {
            magic: MAGIC,
            version: VERSION,
            byte_size: 0,
            nonce: [1, 2, 3, 4],
            echo_pid: 10,
            target_pid: 11,
            input_thread: 12,
            root_pid: 13,
            echo_started: 14,
            target_started: 15,
            root_started: 16,
            root_hwnd: 17,
            input_hwnd: 18,
            focus_generation: 19,
            dll_digest: [0xA5; 32],
        }
    }

    #[test]
    fn layout_is_small_and_fixed() {
        assert_eq!(size_of::<Header>(), 128);
        assert_eq!(size_of::<Mailbox>(), 480);
        assert_eq!(align_of::<Mailbox>(), 8);
        assert!(size_of::<Mailbox>() <= MAX_BYTES);
    }

    #[test]
    fn shape_and_identity_reject_malformed_or_replayed_headers() {
        let mailbox = Mailbox::new(header());
        assert!(header_shape_valid(&mailbox.header));
        assert!(header_identity_matches(&mailbox.header, &mailbox.header));
        let mut wrong_version = mailbox.header;
        wrong_version.version = VERSION + 1;
        assert!(!header_shape_valid(&wrong_version));
        assert!(!header_identity_matches(&mailbox.header, &wrong_version));
        let mut wrong_nonce = mailbox.header;
        wrong_nonce.nonce[0] ^= 1;
        assert!(!header_identity_matches(&mailbox.header, &wrong_nonce));
        let mut wrong_size = mailbox.header;
        wrong_size.byte_size -= 1;
        assert!(!header_shape_valid(&wrong_size));
    }

    #[test]
    fn late_closed_from_an_old_nonce_cannot_match_the_current_session() {
        let current = Mailbox::new(header());
        let mut old = current.header;
        old.nonce[0] ^= 1;
        assert!(!header_identity_matches(&current.header, &old));
    }

    #[test]
    fn roundtrip_busy_and_bounded_reader() {
        let record = AtomicRecord::<4>::new();
        record.publish(&[1, 2, 3, 4]).unwrap();
        assert_eq!(record.snapshot(), Some([1, 2, 3, 4]));
        record.set_revision_for_test(3);
        assert_eq!(record.snapshot(), None);
        assert_eq!(record.publish(&[0; 4]), Err(PublishError::WriterBusy));
    }

    #[test]
    fn rollover_retires_the_mailbox_and_does_not_wrap() {
        let record = AtomicRecord::<1>::new();
        record.set_revision_for_test(u32::MAX - 1);
        assert_eq!(record.publish(&[9]), Err(PublishError::Rollover));
    }

    #[test]
    fn response_replay_requires_the_current_request_sequence() {
        let mut words = [0_u32; 64];
        words[0] = RESPONSE_READY;
        words[1] = 7;
        assert!(response_matches_request(&words, 7));
        assert!(!response_matches_request(&words, 6));
        words[0] = RESPONSE_EMPTY;
        assert!(!response_matches_request(&words, 7));
    }

    #[test]
    fn signed_rectangle_bits_round_trip_without_text_or_pointers() {
        for value in [i32::MIN, -1, 0, 1, i32::MAX] {
            assert_eq!(bits_i32(i32_bits(value)), value);
        }
    }
}
