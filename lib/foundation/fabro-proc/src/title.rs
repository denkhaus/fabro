use std::sync::{Mutex, OnceLock};

struct Buffer {
    start: *mut u8,
    len:   usize,
}

// Safety: after init() we treat the captured argv region as exclusively
// writable by this crate, and serialize writes with the mutex below.
unsafe impl Send for Buffer {}
// Safety: the raw pointer metadata is immutable after capture.
unsafe impl Sync for Buffer {}

static STATE: OnceLock<Mutex<Buffer>> = OnceLock::new();

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn _NSGetArgv() -> *mut *mut *mut libc::c_char;
    fn _NSGetArgc() -> *mut libc::c_int;
}

/// Capture the argv buffer. Call once early in the process.
#[must_use]
pub fn init() -> usize {
    if let Some(state) = STATE.get() {
        return state.lock().map_or(0, |buffer| buffer.len);
    }

    let Some(buffer) = platform_init() else {
        return 0;
    };
    let len = buffer.len;
    let _ = STATE.set(Mutex::new(buffer));

    STATE
        .get()
        .and_then(|state| state.lock().ok().map(|buffer| buffer.len))
        .unwrap_or(len)
}

/// Overwrite the process title shown by `ps`.
pub fn set(title: &str) {
    let Some(state) = STATE.get() else {
        return;
    };
    let Ok(buffer) = state.lock() else {
        return;
    };
    if buffer.start.is_null() || buffer.len == 0 {
        return;
    }

    // SAFETY: init() captured a writable argv byte range for this process, and
    // the mutex guard above provides exclusive access while we rewrite it.
    let dst = unsafe { std::slice::from_raw_parts_mut(buffer.start, buffer.len) };
    write_title(dst, title.as_bytes());
}

fn write_title(dst: &mut [u8], title: &[u8]) {
    if dst.is_empty() {
        return;
    }

    dst.fill(0);
    let copy_len = title.len().min(dst.len().saturating_sub(1));
    dst[..copy_len].copy_from_slice(&title[..copy_len]);
}

#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "process-title initialization reads this process's kernel-owned argv bounds once"
)]
fn platform_init() -> Option<Buffer> {
    // Linux exposes the original argv span independently of libc. In particular,
    // musl does not pass argc/argv to C constructors, unlike glibc.
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let (start, len) = linux_argv_span(&stat)?;
    Some(Buffer {
        // The kernel reports this process's writable, initial argv allocation.
        // No code in this crate relocates or frees that allocation.
        start: std::ptr::with_exposed_provenance_mut(start),
        len,
    })
}

#[cfg(any(target_os = "linux", test))]
fn linux_argv_span(stat: &str) -> Option<(usize, usize)> {
    // comm (field 2) can contain spaces and ')'; the final ')' ends it.
    let (_, tail) = stat.rsplit_once(')')?;
    let mut fields = tail.split_whitespace();
    // The tail starts at field 3; arg_start and arg_end are fields 48 and 49.
    let start: usize = fields.nth(45)?.parse().ok()?;
    let end: usize = fields.next()?.parse().ok()?;
    let len = end.checked_sub(start)?;
    if start == 0 || len == 0 || len > isize::MAX as usize {
        return None;
    }
    Some((start, len))
}

#[cfg(target_os = "macos")]
fn platform_init() -> Option<Buffer> {
    // SAFETY: macOS exposes argc/argv through crt_externs for the current process.
    let argc_ptr = unsafe { _NSGetArgc() };
    // SAFETY: paired with _NSGetArgc above.
    let argv_ptr = unsafe { _NSGetArgv() };
    if argc_ptr.is_null() || argv_ptr.is_null() {
        return None;
    }

    // SAFETY: the pointers above are process globals owned by libc.
    let argc = unsafe { *argc_ptr };
    // SAFETY: same as above.
    let argv = unsafe { *argv_ptr };
    if argc <= 0 || argv.is_null() {
        return None;
    }

    // SAFETY: argc > 0 and argv is non-null, so argv[0] and argv[argc - 1] are
    // valid to read.
    let start = unsafe { *argv };
    // SAFETY: same bound check as above.
    let last = unsafe { *argv.add(usize::try_from(argc).ok()?.saturating_sub(1)) };
    if start.is_null() || last.is_null() {
        return None;
    }

    // SAFETY: last points to a C string owned by the process image.
    let last_len = unsafe { libc::strlen(last) };
    // SAFETY: advancing by the string length plus trailing NUL stays within the
    // captured argv span.
    let end = unsafe { last.add(last_len + 1) };
    let len = (end as usize).checked_sub(start as usize)?;
    if len == 0 {
        return None;
    }

    Some(Buffer {
        start: start.cast(),
        len,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn platform_init() -> Option<Buffer> {
    None
}

#[cfg(test)]
mod tests {
    use super::{linux_argv_span, write_title};

    #[test]
    fn linux_argv_bounds_handle_parentheses_in_the_process_name() {
        let fields = vec!["0"; 45].join(" ");
        let stat = format!("42 (a name ) with (parens)) {fields} 4096 4160 5000 5100 0");
        assert_eq!(linux_argv_span(&stat), Some((4096, 64)));
    }

    #[test]
    fn linux_argv_bounds_refuse_missing_or_invalid_addresses() {
        let fields = vec!["0"; 45].join(" ");
        for addresses in ["", "4096", "0 64", "4096 4096", "4160 4096", "x 4160"] {
            let stat = format!("42 (probe) {fields} {addresses}");
            assert_eq!(linux_argv_span(&stat), None, "{stat}");
        }
    }

    #[test]
    fn write_title_zero_fills_remainder() {
        let mut buffer = [b'x'; 8];
        write_title(&mut buffer, b"fabro");
        assert_eq!(buffer, [b'f', b'a', b'b', b'r', b'o', 0, 0, 0]);
    }

    #[test]
    fn write_title_truncates_to_leave_nul() {
        let mut buffer = [b'x'; 6];
        write_title(&mut buffer, b"toolong");
        assert_eq!(buffer, [b't', b'o', b'o', b'l', b'o', 0]);
    }
}
