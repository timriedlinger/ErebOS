/*
 * A simple logger implementation that writes log messages to the serial port (COM1).
 * The log messages include a timestamp, log level, source file name, and line number.
 *
 * Author: Fabian Ruhland, Heinrich Heine University Duesseldorf, 2026-01-07
 * License: GPLv3
 */

use crate::device::{pit, serial};
use core::fmt::Write;
use log::{Metadata, Record};

/// A simple logger implementing the `log::Log` trait, writing to the serial port (COM1).
pub struct Logger;

impl log::Log for Logger {
    /// Check if the logger is enabled for the given metadata.
    /// This simple implementation always returns true.
    fn enabled(&self, _metadata: &Metadata) -> bool {
        true
    }

    /// Print a log record to the serial port.
    fn log(&self, record: &Record) {
        let file = record
            .file()
            .unwrap_or("unknown") // Try to get the source file path, default to "unknown"
            .split('/') // Split the path by '/'
            .next_back() // Get the last path segment (the file name)
            .unwrap_or("unknown"); // Default to "unknown" if splitting fails

        let line = record.line().unwrap_or(0); // Try to get the line number, default to 0

        let systime = pit::system_time();
        let seconds = systime / 1000;
        let milliseconds = systime % 1000;

        // Format the log message
        let args = format_args!(
            "[{}.{:0>3}][{}][{}@{:0>3}] {}\n",
            seconds,
            milliseconds,
            level_abbreviation(record.level()),
            file,
            line,
            record.args()
        );

        // Write the log message to the serial port
        let mut port = serial::COM1.lock();
        port.write_fmt(args).unwrap(); // Check for formatting errors (should not happen)
    }

    /// Flush the logger.
    /// Since all messages are written immediately, this is a no-op.
    fn flush(&self) {}
}

/// Convert a log level abbreviation to a `log::Level`.
/// Supported abbreviations are:
/// - "TRC" -> Trace
/// - "DBG" -> Debug
/// - "INF" -> Info
/// - "WRN" -> Warn
/// - "ERR" -> Error
/// Returns `None` for unrecognized abbreviations.
pub fn level_from_abbreviation(abbr: &str) -> Option<log::Level> {
    match abbr {
        "TRC" | "trc" => Some(log::Level::Trace),
        "DBG" | "dbg" => Some(log::Level::Debug),
        "INF" | "inf" => Some(log::Level::Info),
        "WRN" | "wrn" => Some(log::Level::Warn),
        "ERR" | "err" => Some(log::Level::Error),
        _ => None,
    }
}

/// Get the three-letter abbreviation for a given log level.
fn level_abbreviation(level: log::Level) -> &'static str {
    match level {
        log::Level::Trace => "TRC",
        log::Level::Debug => "DBG",
        log::Level::Info => "INF",
        log::Level::Warn => "WRN",
        log::Level::Error => "ERR",
    }
}
