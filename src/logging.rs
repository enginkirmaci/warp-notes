//! File logging: readable lines into a daily-rolling file under the suite's
//! shared ~/.warp/logs. Best effort — an unwritable home keeps stderr logging.

use crate::paths::AppPaths;

pub fn initialize() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    warpui_kit_platform::logging::init(&AppPaths::logs_dir(), "warpnotes-")
}
