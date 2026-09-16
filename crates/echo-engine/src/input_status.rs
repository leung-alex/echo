//! Content-free input state shared by native observation and presentation.
use crate::InputTargetGeometry;
use std::time::Instant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputMode {
    Chinese,
    English,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CompositionState {
    Idle,
    Composing,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputAnchor {
    Caret,
    Control,
}

#[derive(Clone, Copy, Debug)]
pub struct InputStatus {
    pub generation: u64,
    pub window: isize,
    pub focused_window: isize,
    pub process: u32,
    pub process_started: u64,
    pub mode: InputMode,
    pub composition: CompositionState,
    pub anchor: InputAnchor,
    pub geometry: InputTargetGeometry,
    pub sampled_at: Instant,
}
