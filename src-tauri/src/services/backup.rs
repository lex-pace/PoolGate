//! Database backup and restore service.
//!
//! Provides manual backup/restore as well as an automatic weekly backup
//! that keeps the last 4 backup files.

use chrono::Local;
use std::fs;
use std::path::Path;

/// Create a timestamped backup copy of the database.
///
/// The backup file is named `gateway_YYYYMMDD_HHMMSS.db` and placed in
/// `backup_dir`. Both `db_path` and `backup_dir` are resolved from the
/// filesystem.
///
/// Returns the full path of the created backup on success.
pub fn backup_database(db_path: &str, backup_dir: &str) -> Result<String, String> {
    // Ensure the backup directory exists
    fs::create_dir_all(backup_dir).map_err(|e| format!("Cannot create backup directory: {}", e))?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let backup_path = format!("{}/gateway_{}.db", backup_dir, timestamp);

    fs::copy(db_path, &backup_path).map_err(|e| format!("Backup copy failed: {}", e))?;

    tracing::info!("Database backed up to {}", backup_path);
    Ok(backup_path)
}

/// Restore the database from a backup file.
///
/// This is a direct copy operation: the backup file is copied back to
/// `db_path`. The application should be **stopped** before calling this
/// to avoid database corruption.
pub fn restore_database(backup_path: &str, db_path: &str) -> Result<(), String> {
    // Verify the backup exists
    if !Path::new(backup_path).exists() {
        return Err(format!("Backup file not found: {}", backup_path));
    }

    fs::copy(backup_path, db_path).map_err(|e| format!("Restore copy failed: {}", e))?;

    tracing::info!("Database restored from {}", backup_path);
    Ok(())
}

/// Run a weekly auto-backup loop.
///
/// This spawns a background task that:
/// - Creates a backup every 7 days (168 hours).
/// - Keeps only the last 4 backups — older ones are automatically pruned.
///
/// Call this once at application startup. The first backup runs after the
/// initial delay.
pub fn auto_backup(db_path: &str, backup_dir: &str) {
    let db_path = db_path.to_string();
    let backup_dir = backup_dir.to_string();

    std::thread::spawn(move || {
        tracing::info!("Auto-backup service started (weekly, keep last 4)");

        loop {
            // Sleep for 7 days
            std::thread::sleep(std::time::Duration::from_secs(7 * 24 * 3600));

            if let Err(e) = backup_database(&db_path, &backup_dir) {
                tracing::error!("Auto-backup failed: {}", e);
                continue;
            }

            // Prune old backups, keep only the 4 most recent
            if let Err(e) = prune_backups(&backup_dir, 4) {
                tracing::warn!("Auto-backup pruning failed: {}", e);
            }
        }
    });
}

/// Keep only the `keep` most recent backup files in `backup_dir`.
/// Older files matching the pattern `gateway_*.db` are deleted.
fn prune_backups(backup_dir: &str, keep: usize) -> Result<(), String> {
    let dir = Path::new(backup_dir);
    if !dir.exists() {
        return Ok(());
    }

    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("Cannot read backup dir: {}", e))?
        .filter_map(|entry| {
            let e = entry.ok()?;
            let path = e.path();
            let file_name = path.file_name()?.to_str()?;
            if file_name.starts_with("gateway_") && file_name.ends_with(".db") {
                let modified = e.metadata().ok()?.modified().ok()?;
                Some((modified, path))
            } else {
                None
            }
        })
        .collect();

    // Sort by modification time (newest first)
    entries.sort_by(|a, b| b.0.cmp(&a.0));

    // Remove entries beyond `keep`
    if entries.len() > keep {
        for (_mtime, path) in entries.iter().skip(keep) {
            fs::remove_file(path)
                .map_err(|e| format!("Cannot remove old backup {:?}: {}", path, e))?;
            tracing::info!("Pruned old backup: {:?}", path);
        }
    }

    Ok(())
}
