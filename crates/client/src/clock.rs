use std::{
    mem::MaybeUninit,
    time::{SystemTime, UNIX_EPOCH},
};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn local(seconds: u64) -> libc::tm {
    let seconds = seconds as libc::time_t;
    let mut tm = MaybeUninit::zeroed();
    // SAFETY: both pointers are valid, and localtime_r keeps no reference to them.
    unsafe { libc::localtime_r(&seconds, tm.as_mut_ptr()) };
    // SAFETY: zeroed is a valid tm, whether or not localtime_r filled it in.
    unsafe { tm.assume_init() }
}

// A moment in milliseconds since the Unix epoch, in local time: just the time today, the date too otherwise.
pub fn stamp(millis: u64) -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (then, now) = (local(millis / 1000), local(now));
    let time = format!("{:02}:{:02}", then.tm_hour, then.tm_min);
    let month = MONTHS[then.tm_mon.clamp(0, 11) as usize];
    if (then.tm_year, then.tm_yday) == (now.tm_year, now.tm_yday) {
        time
    } else if then.tm_year == now.tm_year {
        format!("{month} {} {time}", then.tm_mday)
    } else {
        format!("{month} {} {} {time}", then.tm_mday, then.tm_year + 1900)
    }
}
