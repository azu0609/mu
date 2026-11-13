// Token counts use decimal units, not binary byte units.
const UNITS: [(char, u64); 6] = [
    ('k', 1_000),
    ('M', 1_000_000),
    ('B', 1_000_000_000),
    ('T', 1_000_000_000_000),
    ('P', 1_000_000_000_000_000),
    ('E', 1_000_000_000_000_000_000),
];

pub fn compact(n: u64) -> String {
    let (mut suffix, mut multiplier) = UNITS[0];
    if n < multiplier {
        return n.to_string();
    }
    for &(next_suffix, next_multiplier) in UNITS.iter().skip(1) {
        // Promote just before rounding would show 1000.0 of the smaller unit.
        if n < next_multiplier - multiplier / 20 {
            break;
        }
        (suffix, multiplier) = (next_suffix, next_multiplier);
    }
    format!("{:.1}{suffix}", n as f64 / multiplier as f64)
}

// The parsing direction must be exact: fractional inputs cannot silently
// round down, and overflowing counts are invalid.
pub fn parse(text: &str) -> Option<u64> {
    let suffix = text.chars().last()?;
    let multiplier = UNITS
        .iter()
        .find(|(unit, _)| unit.eq_ignore_ascii_case(&suffix))
        .or_else(|| (suffix == 'g' || suffix == 'G').then(|| &UNITS[2]))
        .map(|&(_, multiplier)| multiplier);
    let (number, multiplier) = match multiplier {
        Some(multiplier) => (text.strip_suffix(suffix)?, multiplier),
        None => (text, 1),
    };
    let value = if let Some((whole, fraction)) = number.split_once('.') {
        if multiplier == 1 || fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let fraction = fraction.trim_end_matches('0');
        let power = multiplier.ilog10();
        if fraction.len() > power as usize {
            return None;
        }
        let whole = if whole.is_empty() { 0 } else { whole.parse::<u64>().ok()? };
        let fractional = if fraction.is_empty() { 0 } else { fraction.parse::<u64>().ok()? };
        whole.checked_mul(multiplier)?.checked_add(fractional.checked_mul(10u64.pow(power - fraction.len() as u32))?)?
    } else {
        number.parse::<u64>().ok()?.checked_mul(multiplier)?
    };
    (value > 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{compact, parse};

    #[test]
    fn parses_context_counts() {
        for (input, expected) in [
            ("128000", 128_000),
            ("128k", 128_000),
            ("1.5M", 1_500_000),
            (".5k", 500),
            ("1.0000k", 1_000),
            ("2G", 2_000_000_000),
            ("2b", 2_000_000_000),
            ("1t", 1_000_000_000_000),
            ("1p", 1_000_000_000_000_000),
            ("1e", 1_000_000_000_000_000_000),
            ("18446744073709551615", u64::MAX),
        ] {
            assert_eq!(parse(input), Some(expected), "{input}");
        }
    }

    #[test]
    fn rejects_invalid_context_counts() {
        for input in [
            "",
            "0",
            "0k",
            "-1k",
            "k",
            "1x",
            "1.5",
            "1.k",
            "1.0001k",
            "0.0001k",
            "1.2.3k",
            "18446744073709551616",
            "18446744073709551616k",
            "20e",
        ] {
            assert_eq!(parse(input), None, "{input}");
        }
    }

    #[test]
    fn compact_status_counts() {
        for (count, expected) in [
            (0, "0"),
            (999, "999"),
            (1_000, "1.0k"),
            (15_500, "15.5k"),
            (999_949, "999.9k"),
            (999_950, "1.0M"),
            (1_000_000, "1.0M"),
            (1_000_000_000, "1.0B"),
            (1_000_000_000_000, "1.0T"),
        ] {
            assert_eq!(compact(count), expected, "{count}");
        }
    }
}
