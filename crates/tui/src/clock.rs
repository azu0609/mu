use std::{
    ffi::CStr,
    time::{SystemTime, UNIX_EPOCH},
};

fn local(seconds: u64) -> libc::tm {
    // SAFETY: an all-zero tm is valid, and localtime_r keeps no reference to either pointer.
    unsafe {
        let mut tm = std::mem::zeroed();
        libc::localtime_r(&(seconds as libc::time_t), &mut tm);
        tm
    }
}

fn strftime(tm: &libc::tm, pattern: &CStr) -> String {
    let mut buf = [0u8; 32];
    // SAFETY: buf is writable for its whole length, and strftime never writes past it.
    let len = unsafe { libc::strftime(buf.as_mut_ptr().cast(), buf.len(), pattern.as_ptr(), tm) };
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

// A moment in milliseconds since the Unix epoch, in local time: just the time today, the date too otherwise.
pub fn stamp(millis: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (then, now) = (local(millis / 1000), local(now));
    let pattern = if (then.tm_year, then.tm_yday) == (now.tm_year, now.tm_yday) {
        c"%H:%M"
    } else if then.tm_year == now.tm_year {
        c"%b %-d %H:%M"
    } else {
        c"%b %-d %Y %H:%M"
    };
    strftime(&then, pattern)
}
