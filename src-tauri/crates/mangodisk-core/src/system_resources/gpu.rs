//! Automatic selection uses the busiest readable GPU; manual selection never falls back.
use mangodisk_platform::system_resources::gpu::GpuAdapterUsage;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuUsage {
    pub used_percent: f64,
    pub adapter_id: String,
    pub adapter_name: String,
    pub details: Option<mangodisk_platform::system_resources::gpu::details::GpuDetails>,
}

pub fn select_adapter_index(adapters: &[GpuAdapterUsage], selected: Option<&str>) -> Option<usize> {
    adapters
        .iter()
        .enumerate()
        .filter(|(_, value)| {
            selected.is_none_or(|id| id == value.id)
                && value.used_percent.is_finite()
                && (0.0..=100.0).contains(&value.used_percent)
        })
        .max_by(|(_, a), (_, b)| {
            a.used_percent
                .total_cmp(&b.used_percent)
                .then_with(|| b.id.cmp(&a.id))
        })
        .map(|(index, _)| index)
}

pub fn select(mut adapters: Vec<GpuAdapterUsage>, selected: Option<&str>) -> Option<GpuUsage> {
    let index = select_adapter_index(&adapters, selected)?;
    let value = adapters.swap_remove(index);
    Some(GpuUsage {
        used_percent: value.used_percent,
        adapter_id: value.id,
        adapter_name: value.name,
        details: value.details,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter(id: &str, used_percent: f64) -> GpuAdapterUsage {
        GpuAdapterUsage {
            id: id.into(),
            name: format!("GPU {id}"),
            used_percent,
            details: None,
        }
    }
    #[test]
    fn summary_uses_the_busiest_valid_device_with_stable_ties() {
        let result = select(
            vec![
                adapter("b", 35.0),
                adapter("a", 35.0),
                adapter("c", f64::NAN),
            ],
            None,
        )
        .unwrap();
        assert_eq!(result.adapter_id, "a");
        assert_eq!(result.used_percent, 35.0);
        assert_eq!(
            select(vec![adapter("a", 10.0), adapter("b", 85.0)], None)
                .unwrap()
                .adapter_id,
            "b"
        );
    }
    #[test]
    fn fixed_selection_does_not_switch_to_another_busy_device() {
        let result = select(vec![adapter("a", 3.0), adapter("b", 56.0)], Some("a")).unwrap();
        assert_eq!(result.adapter_id, "a");
        assert_eq!(result.used_percent, 3.0);
        assert!(select(vec![adapter("b", 56.0)], Some("a")).is_none());
        assert!(select(vec![adapter("a", f64::NAN), adapter("b", 56.0)], Some("a")).is_none());
    }
    #[test]
    fn unknown_activity_is_distinct_from_real_zero() {
        assert!(select(vec![], None).is_none());
        assert!(select(vec![adapter("a", -1.0), adapter("b", 101.0)], None).is_none());
        assert_eq!(
            select(vec![adapter("a", 0.0)], None).unwrap().used_percent,
            0.0
        );
    }
}
