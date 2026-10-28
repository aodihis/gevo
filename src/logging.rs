use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub struct LogConfig {
    pub enabled: bool,
    pub dir: PathBuf,
    pub keep_days: u32,
    pub level: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: default_log_dir(),
            keep_days: 7,
            level: "info".into(),
        }
    }
}

fn default_log_dir() -> PathBuf {
    std::env::temp_dir().join("gevo").join("logs")
}

pub fn sweep_old_logs(dir: &Path, keep_days: u32, now: SystemTime) -> io::Result<usize> {
    if !dir.exists() {
        return Ok(0);
    }
    let max_age = Duration::from_secs(u64::from(keep_days) * 86_400);
    let mut removed = 0usize;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified()?;
        if let Ok(age) = now.duration_since(modified)
            && age > max_age
        {
            fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn make_env_filter(level: &str) -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level))
}

pub fn init_logging(cfg: &LogConfig) -> Option<WorkerGuard> {
    let filter = make_env_filter(&cfg.level);

    if !cfg.enabled {
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt::layer())
            .try_init();
        return None;
    }

    let _ = fs::create_dir_all(&cfg.dir);
    let _ = sweep_old_logs(&cfg.dir, cfg.keep_days, SystemTime::now());

    let appender = tracing_appender::rolling::daily(&cfg.dir, "gevo.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer())
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .try_init();

    Some(guard)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn default_is_disabled_with_seven_day_retention() {
        let c = LogConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.keep_days, 7);
        assert_eq!(c.level, "info");
    }

    #[test]
    fn sweep_on_missing_dir_returns_zero() {
        let missing = std::env::temp_dir().join("gevo-sweep-does-not-exist-xyz");
        let _ = fs::remove_dir_all(&missing);
        let removed = sweep_old_logs(&missing, 7, SystemTime::now()).unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn sweep_removes_files_older_than_keep_days() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("gevo.log.old");
        File::create(&f).unwrap().write_all(b"x").unwrap();

        // Treat "now" as 10 days in the future so the just-created file is 10 days old.
        let future_now = SystemTime::now() + Duration::from_secs(10 * 86_400);
        let removed = sweep_old_logs(dir.path(), 7, future_now).unwrap();
        assert_eq!(removed, 1);
        assert!(!f.exists());
    }

    #[test]
    fn sweep_keeps_files_within_retention() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("gevo.log.today");
        File::create(&f).unwrap().write_all(b"x").unwrap();

        let removed = sweep_old_logs(dir.path(), 7, SystemTime::now()).unwrap();
        assert_eq!(removed, 0);
        assert!(f.exists());
    }

    #[test]
    fn sweep_ignores_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();

        let removed = sweep_old_logs(dir.path(), 0, SystemTime::now()).unwrap();
        assert_eq!(removed, 0);
        assert!(sub.exists());
    }

    #[test]
    fn env_filter_falls_back_to_configured_level() {
        unsafe {
            std::env::remove_var("RUST_LOG");
        }
        let f = make_env_filter("warn");
        assert!(f.to_string().contains("warn"));
    }
}
