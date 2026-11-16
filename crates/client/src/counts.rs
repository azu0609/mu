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
