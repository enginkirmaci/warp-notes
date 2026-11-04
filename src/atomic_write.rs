//! The atomic file replace the note writer rides: the platform kit's unique
//! temp file + rename in the target directory, so a crash mid-write never
//! leaves a truncated note behind.

pub use warpui_kit_platform::io::write_atomically;
