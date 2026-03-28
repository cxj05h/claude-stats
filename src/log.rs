use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{Local, NaiveDate};

static LOGGER: Mutex<Option<Logger>> = Mutex::new(None);

struct Logger {
    file: File,
    path: PathBuf,
    date: NaiveDate,
}

fn log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("stats.log")
}

/// Initialize the logger. Rotates (truncates) if the log is from a previous day.
pub fn init() {
    let path = log_path();
    let today = Local::now().date_naive();

    // Check if existing log is from a previous day → rotate
    if path.exists() {
        if let Ok(meta) = fs::metadata(&path) {
            if let Ok(modified) = meta.modified() {
                let mod_date: chrono::DateTime<Local> = modified.into();
                if mod_date.date_naive() < today {
                    // Rotate: just truncate (single day retention)
                    let _ = fs::write(&path, "");
                }
            }
        }
    }

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);

    if let Ok(mut f) = file {
        let _ = writeln!(f, "--- claude-stats started {} ---", Local::now().format("%Y-%m-%d %H:%M:%S"));
        let mut guard = LOGGER.lock().unwrap();
        *guard = Some(Logger {
            file: f,
            path,
            date: today,
        });
    }
}

/// Log a message with timestamp.
pub fn log(msg: &str) {
    let mut guard = match LOGGER.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    if let Some(logger) = guard.as_mut() {
        // Daily rotation check
        let today = Local::now().date_naive();
        if today > logger.date {
            let _ = fs::write(&logger.path, "");
            if let Ok(f) = OpenOptions::new().create(true).append(true).open(&logger.path) {
                logger.file = f;
                logger.date = today;
            }
        }
        let ts = Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(logger.file, "[{}] {}", ts, msg);
    }
}

/// Log a formatted message (convenience macro-like function).
pub fn logf(args: std::fmt::Arguments<'_>) {
    log(&args.to_string());
}

/// Convenience macro for formatted logging.
#[macro_export]
macro_rules! cs_log {
    ($($arg:tt)*) => {
        $crate::log::logf(format_args!($($arg)*))
    };
}
