const MAX_THREADS: usize = 4;
fn choose(available: usize) -> usize {
    available.saturating_sub(1).clamp(1, MAX_THREADS)
}
pub(super) fn threads() -> usize {
    choose(std::thread::available_parallelism().map_or(1, |n| n.get()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_parallelism_reserves_a_logical_cpu_when_possible() {
        for (available, expected) in [
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 3),
            (5, 4),
            (64, 4),
            (usize::MAX, 4),
        ] {
            assert_eq!(choose(available), expected);
        }
    }
    #[test]
    fn normal_production_plan_uses_the_bounded_count() {
        use crate::{
            claurst_local_runtime::ClaurstLocalRuntimePlan, local_model_catalog::LocalModelCatalog,
        };
        let catalog = LocalModelCatalog::shipped().unwrap();
        let plan = ClaurstLocalRuntimePlan::build(
            super::super::tests::request(),
            catalog.model("qwen3-8b-q4-k-m").unwrap(),
            18080,
        )
        .unwrap();
        let actual = plan.model_configuration["threads"].as_u64().unwrap();
        assert_eq!(actual, threads() as u64);
        assert!((1..=4).contains(&actual));
    }
}
