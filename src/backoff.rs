use std::time::Duration;

use chrono::{DateTime, Utc};

/// Délais après la 1re, 2e, 3e, 4e et 5e tentative échouée.
/// La 6e tentative (quand `max_attempts` vaut 6) ne planifie plus de retry.
const DELAYS_SECS: [u64; 5] = [30, 120, 600, 3_600, 21_600];

/// Délai avant la prochaine tentative après `attempt_count` échecs déjà consommés.
///
/// `attempt_count` est le nombre de tentatives terminées (1 = le premier POST a échoué).
/// `None` signifie que le budget est épuisé.
pub fn retry_delay(attempt_count: u32, max_attempts: u32) -> Option<Duration> {
    if attempt_count == 0 || attempt_count >= max_attempts {
        return None;
    }
    let index = (attempt_count as usize - 1).min(DELAYS_SECS.len() - 1);
    Some(Duration::from_secs(DELAYS_SECS[index]))
}

pub fn next_attempt_at(
    now: DateTime<Utc>,
    attempt_count: u32,
    max_attempts: u32,
) -> Option<DateTime<Utc>> {
    let delay = retry_delay(attempt_count, max_attempts)?;
    let seconds = i64::try_from(delay.as_secs()).ok()?;
    now.checked_add_signed(chrono::Duration::seconds(seconds))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::{TimeZone, Utc};

    use super::{next_attempt_at, retry_delay};

    #[test]
    fn follows_the_documented_schedule_then_exhausts() {
        assert_eq!(retry_delay(1, 6), Some(Duration::from_secs(30)));
        assert_eq!(retry_delay(2, 6), Some(Duration::from_secs(120)));
        assert_eq!(retry_delay(3, 6), Some(Duration::from_secs(600)));
        assert_eq!(retry_delay(4, 6), Some(Duration::from_secs(3_600)));
        assert_eq!(retry_delay(5, 6), Some(Duration::from_secs(21_600)));
        assert_eq!(retry_delay(6, 6), None);
    }

    #[test]
    fn rejects_zero_and_honours_a_smaller_budget() {
        assert_eq!(retry_delay(0, 6), None);
        assert_eq!(retry_delay(1, 3), Some(Duration::from_secs(30)));
        assert_eq!(retry_delay(2, 3), Some(Duration::from_secs(120)));
        assert_eq!(retry_delay(3, 3), None);
    }

    #[test]
    fn repeats_the_last_delay_when_the_budget_exceeds_the_schedule() {
        assert_eq!(retry_delay(8, 10), Some(Duration::from_secs(21_600)));
        assert_eq!(retry_delay(10, 10), None);
    }

    #[test]
    fn next_attempt_is_now_plus_the_delay() {
        let now = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();
        let next = next_attempt_at(now, 1, 6).unwrap();
        assert_eq!((next - now).num_seconds(), 30);
        assert!(next_attempt_at(now, 6, 6).is_none());
    }
}
