//! A windowed app has no console, so `eprintln!` goes nowhere. Pointing stderr at a file keeps those
//! messages for when something needs explaining ("why didn't it offer to record?").

use std::fs::OpenOptions;
use std::os::windows::io::AsRawHandle;
use std::path::Path;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};

/// Start a fresh log once it passes this size.
const MAX: u64 = 1 << 20;

/// Send stderr to `path` (appending; restarted once it grows past 1 MB).
pub fn redirect_stderr(path: &Path) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let fresh = std::fs::metadata(path).map_or(false, |m| m.len() > MAX);
    let file = OpenOptions::new().create(true).append(!fresh).write(true).truncate(fresh).open(path)?;
    unsafe { SetStdHandle(STD_ERROR_HANDLE, HANDLE(file.as_raw_handle()))? };
    // stderr keeps the handle for the life of the process
    std::mem::forget(file);
    Ok(())
}
