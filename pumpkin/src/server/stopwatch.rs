use pumpkin_util::identifier::Identifier;
use pumpkin_world::world_info::data_files::StopwatchesData;
use rustc_hash::FxHashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug)]
struct Stopwatch {
    started_at: Instant,
    accumulated: Duration,
}

impl Stopwatch {
    fn new() -> Self {
        Self {
            started_at: Instant::now(),
            accumulated: Duration::ZERO,
        }
    }

    fn from_elapsed_milliseconds(milliseconds: i64) -> Self {
        Self {
            started_at: Instant::now(),
            accumulated: Duration::from_millis(milliseconds.max(0) as u64),
        }
    }

    fn elapsed(self) -> Duration {
        self.accumulated.saturating_add(self.started_at.elapsed())
    }
}

pub struct StopwatchManager {
    stopwatches: RwLock<FxHashMap<Identifier, Stopwatch>>,
}

impl StopwatchManager {
    #[must_use]
    pub fn new(data: &StopwatchesData) -> Self {
        let stopwatches = data
            .stopwatches
            .iter()
            .filter_map(|(id, milliseconds)| {
                Identifier::parse(id)
                    .ok()
                    .map(|id| (id, Stopwatch::from_elapsed_milliseconds(*milliseconds)))
            })
            .collect();
        Self {
            stopwatches: RwLock::new(stopwatches),
        }
    }

    pub async fn create(&self, id: Identifier) -> bool {
        let mut stopwatches = self.stopwatches.write().await;
        if let std::collections::hash_map::Entry::Vacant(entry) = stopwatches.entry(id) {
            entry.insert(Stopwatch::new());
            return true;
        }
        false
    }

    pub async fn elapsed_seconds(&self, id: &Identifier) -> Option<f64> {
        self.stopwatches
            .read()
            .await
            .get(id)
            .copied()
            .map(|stopwatch| stopwatch.elapsed().as_secs_f64())
    }

    pub async fn restart(&self, id: &Identifier) -> bool {
        let mut stopwatches = self.stopwatches.write().await;
        let Some(stopwatch) = stopwatches.get_mut(id) else {
            return false;
        };
        *stopwatch = Stopwatch::new();
        true
    }

    pub async fn remove(&self, id: &Identifier) -> bool {
        self.stopwatches.write().await.remove(id).is_some()
    }

    pub async fn ids(&self) -> Vec<Identifier> {
        let mut ids = self
            .stopwatches
            .read()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub async fn snapshot(&self, data_version: i32) -> StopwatchesData {
        let stopwatches = self
            .stopwatches
            .read()
            .await
            .iter()
            .map(|(id, stopwatch)| {
                let milliseconds =
                    i64::try_from(stopwatch.elapsed().as_millis()).unwrap_or(i64::MAX);
                (id.to_string(), milliseconds)
            })
            .collect();
        StopwatchesData {
            stopwatches,
            data_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StopwatchManager;
    use pumpkin_util::identifier::Identifier;
    use pumpkin_world::world_info::data_files::StopwatchesData;

    #[tokio::test]
    async fn restored_elapsed_time_excludes_offline_time_and_can_restart() {
        let id = Identifier::parse("test:timer").unwrap();
        let data = StopwatchesData {
            stopwatches: std::iter::once((id.to_string(), 2_500)).collect(),
            data_version: 0,
        };
        let manager = StopwatchManager::new(&data);

        let elapsed = manager.elapsed_seconds(&id).await.unwrap();
        assert!((2.5..2.6).contains(&elapsed));
        assert!(manager.restart(&id).await);
        assert!(manager.elapsed_seconds(&id).await.unwrap() < 0.1);
    }

    #[tokio::test]
    async fn duplicate_create_fails_and_snapshot_keeps_elapsed_time() {
        let manager = StopwatchManager::new(&StopwatchesData::default());
        let id = Identifier::parse("test:timer").unwrap();
        assert!(manager.create(id.clone()).await);
        assert!(!manager.create(id.clone()).await);

        let snapshot = manager.snapshot(4903).await;
        assert_eq!(snapshot.data_version, 4903);
        assert!(snapshot.stopwatches.contains_key(&id.to_string()));
    }
}
