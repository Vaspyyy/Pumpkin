use crate::command::CommandSender;
use crate::server::Server;
use pumpkin_util::identifier::Identifier;
use pumpkin_world::world_info::data_files::{
    ScheduledEventCallbackData, ScheduledEventData, ScheduledEventsData,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use tokio::sync::Mutex;
use tracing::warn;

const FUNCTION_CALLBACK: &str = "minecraft:function";
const FUNCTION_TAG_CALLBACK: &str = "minecraft:function_tag";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduledFunctionKind {
    Function,
    Tag,
}

impl ScheduledFunctionKind {
    const fn callback_type(self) -> &'static str {
        match self {
            Self::Function => FUNCTION_CALLBACK,
            Self::Tag => FUNCTION_TAG_CALLBACK,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ScheduledFunction {
    trigger_time: i64,
    sequence: u64,
    name: String,
    id: Identifier,
    kind: ScheduledFunctionKind,
}

pub struct FunctionScheduler {
    events: Mutex<Vec<ScheduledFunction>>,
    game_time: AtomicI64,
    next_sequence: AtomicU64,
}

impl FunctionScheduler {
    #[must_use]
    pub fn new(data: &ScheduledEventsData, game_time: i64) -> Self {
        let mut events = Vec::with_capacity(data.events.len());
        for (sequence, event) in data.events.iter().enumerate() {
            let kind = match event.callback.callback_type.as_str() {
                FUNCTION_CALLBACK | "function" => ScheduledFunctionKind::Function,
                FUNCTION_TAG_CALLBACK | "function_tag" => ScheduledFunctionKind::Tag,
                callback_type => {
                    warn!("Ignoring unsupported scheduled event callback {callback_type}");
                    continue;
                }
            };
            let Ok(id) = Identifier::parse(&event.callback.id) else {
                warn!(
                    "Ignoring scheduled event {} with invalid identifier {}",
                    event.id, event.callback.id
                );
                continue;
            };
            events.push(ScheduledFunction {
                trigger_time: event.trigger_time,
                sequence: sequence as u64,
                name: event.id.clone(),
                id,
                kind,
            });
        }

        Self {
            next_sequence: AtomicU64::new(events.len() as u64),
            events: Mutex::new(events),
            game_time: AtomicI64::new(game_time),
        }
    }

    #[must_use]
    pub fn current_time(&self) -> i64 {
        self.game_time.load(Ordering::Relaxed)
    }

    pub async fn schedule(
        &self,
        id: Identifier,
        kind: ScheduledFunctionKind,
        delay: i32,
        replace: bool,
    ) -> i64 {
        let trigger_time = self.current_time().saturating_add(i64::from(delay));
        let name = match kind {
            ScheduledFunctionKind::Function => id.to_string(),
            ScheduledFunctionKind::Tag => format!("#{id}"),
        };
        let mut events = self.events.lock().await;
        if replace {
            events.retain(|event| event.name != name);
        }
        if !events
            .iter()
            .any(|event| event.name == name && event.trigger_time == trigger_time)
        {
            events.push(ScheduledFunction {
                trigger_time,
                sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
                name,
                id,
                kind,
            });
        }
        trigger_time
    }

    pub async fn clear(&self, name: &str) -> usize {
        let mut events = self.events.lock().await;
        let previous_len = events.len();
        events.retain(|event| event.name != name);
        previous_len - events.len()
    }

    pub async fn event_names(&self) -> Vec<String> {
        let mut names = self
            .events
            .lock()
            .await
            .iter()
            .map(|event| event.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }

    pub async fn snapshot(&self, data_version: i32) -> ScheduledEventsData {
        let mut events = self.events.lock().await.clone();
        events.sort_by_key(|event| (event.trigger_time, event.sequence));
        ScheduledEventsData {
            data_version,
            events: events
                .into_iter()
                .map(|event| ScheduledEventData {
                    trigger_time: event.trigger_time,
                    id: event.name,
                    callback: ScheduledEventCallbackData {
                        callback_type: event.kind.callback_type().to_string(),
                        id: event.id.to_string(),
                    },
                })
                .collect(),
        }
    }

    pub async fn tick(&self, server: &Arc<Server>) {
        let game_time = self.game_time.fetch_add(1, Ordering::Relaxed) + 1;
        let due = {
            let mut events = self.events.lock().await;
            events.sort_by_key(|event| (event.trigger_time, event.sequence));
            let due_count = events.partition_point(|event| event.trigger_time <= game_time);
            events.drain(..due_count).collect::<Vec<_>>()
        };
        if due.is_empty() {
            return;
        }

        let source = CommandSender::Dummy.into_source(server).await;
        for event in due {
            let result = match event.kind {
                ScheduledFunctionKind::Function => {
                    server
                        .data_pack_manager
                        .execute_function(server, &event.id, &source)
                        .await
                }
                ScheduledFunctionKind::Tag => {
                    server
                        .data_pack_manager
                        .execute_tag(server, &event.id, &source)
                        .await
                }
            };
            if let Err(error) = result {
                warn!("Failed to execute scheduled event {}: {error}", event.name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FunctionScheduler, ScheduledFunctionKind};
    use pumpkin_util::identifier::Identifier;
    use pumpkin_world::world_info::data_files::ScheduledEventsData;

    #[tokio::test]
    async fn replace_and_append_match_vanilla_identity_rules() {
        let scheduler = FunctionScheduler::new(&ScheduledEventsData::default(), 100);
        let id = Identifier::parse("test:later").unwrap();

        scheduler
            .schedule(id.clone(), ScheduledFunctionKind::Function, 20, false)
            .await;
        scheduler
            .schedule(id.clone(), ScheduledFunctionKind::Function, 30, false)
            .await;
        scheduler
            .schedule(id, ScheduledFunctionKind::Function, 40, true)
            .await;

        let snapshot = scheduler.snapshot(0).await;
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].trigger_time, 140);
        assert_eq!(snapshot.events[0].id, "test:later");
    }

    #[tokio::test]
    async fn duplicate_name_and_time_is_ignored() {
        let scheduler = FunctionScheduler::new(&ScheduledEventsData::default(), 0);
        let id = Identifier::parse("test:later").unwrap();
        scheduler
            .schedule(id.clone(), ScheduledFunctionKind::Function, 20, false)
            .await;
        scheduler
            .schedule(id, ScheduledFunctionKind::Function, 20, false)
            .await;
        assert_eq!(scheduler.snapshot(0).await.events.len(), 1);
    }
}
