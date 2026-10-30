//! Numeric gating preserves cadence; transport recovery backs off within a fixed budget.
use crate::PlatformErrorCode;

pub(super) fn delay_ms(error: PlatformErrorCode, failures: u8, interval_ms: u64) -> u64 {
    match error {
        PlatformErrorCode::InvalidData => interval_ms,
        PlatformErrorCode::Unsupported
        | PlatformErrorCode::AccessDenied
        | PlatformErrorCode::UserCancelled => 30_000,
        _ => interval_ms
            .saturating_mul(1u64 << failures.saturating_sub(1).min(3))
            .min(30_000),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gated_samples_stay_regular_and_transport_retries_remain_bounded() {
        for count in [0, 1, 2, 100, u8::MAX] {
            assert_eq!(delay_ms(PlatformErrorCode::InvalidData, count, 4000), 4000);
            assert_eq!(delay_ms(PlatformErrorCode::Unsupported, count, 4000), 30000);
        }
        assert_eq!(
            (1..=5)
                .map(|count| delay_ms(PlatformErrorCode::Io, count, 4000))
                .collect::<Vec<_>>(),
            vec![4000, 8000, 16000, 30000, 30000]
        );
    }
}
