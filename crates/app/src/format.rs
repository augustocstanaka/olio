//! How times are written on tiles.

/// "0:21.4" under a minute, "1:53" under an hour, "1:02:03" beyond.
/// Truncates rather than rounds, so a clip never shows longer than it is.
pub fn clock(seconds: f64) -> String {
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    if seconds < 60.0 {
        let tenths = (seconds * 10.0).floor() as u64;
        format!("0:{:02}.{}", tenths / 10, tenths % 10)
    } else {
        let whole = seconds.floor() as u64;
        if whole < 3600 {
            format!("{}:{:02}", whole / 60, whole % 60)
        } else {
            format!("{}:{:02}:{:02}", whole / 3600, whole / 60 % 60, whole % 60)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::clock;

    #[test]
    fn times_read_like_farrago() {
        assert_eq!(clock(0.0), "0:00.0");
        assert_eq!(clock(21.46), "0:21.4");
        assert_eq!(clock(59.99), "0:59.9");
        assert_eq!(clock(113.2), "1:53");
        assert_eq!(clock(3723.0), "1:02:03");
        assert_eq!(clock(-1.0), "0:00.0");
        assert_eq!(clock(f64::NAN), "0:00.0");
    }
}
