use super::recipe::parse_item_stack;
use pumpkin_data::advancement_data::FrameType;
use pumpkin_protocol::java::client::play::{ClientAdvancement, ClientAdvancementDisplay};
use pumpkin_util::identifier::Identifier;
use pumpkin_util::text::TextComponent;
use rustc_hash::FxHashMap;
use serde::Deserialize;

#[derive(Deserialize)]
struct RawAdvancement {
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    display: Option<RawDisplay>,
    criteria: FxHashMap<String, serde_json::Value>,
    #[serde(default)]
    requirements: Option<Vec<Vec<String>>>,
    #[serde(default, alias = "sends_telemetry_event")]
    send_telemetry: bool,
}

#[derive(Deserialize)]
struct RawDisplay {
    title: TextComponent,
    description: TextComponent,
    icon: serde_json::Value,
    #[serde(default)]
    frame: RawFrame,
    #[serde(default)]
    background: Option<String>,
    #[serde(default = "default_true")]
    show_toast: bool,
    #[serde(default)]
    hidden: bool,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawFrame {
    #[default]
    Task,
    Challenge,
    Goal,
}

const fn default_true() -> bool {
    true
}

pub(super) fn parse_advancement(
    id: &Identifier,
    contents: &[u8],
) -> Result<ClientAdvancement, String> {
    let raw: RawAdvancement = serde_json::from_slice(contents)
        .map_err(|error| format!("advancement is not valid JSON: {error}"))?;
    if raw.criteria.is_empty() {
        return Err("advancement must define at least one criterion".to_string());
    }
    let parent = raw
        .parent
        .map(|parent| {
            Identifier::parse(&parent)
                .map_err(|error| format!("invalid parent identifier {parent:?}: {error}"))
        })
        .transpose()?;
    let requirements = raw.requirements.unwrap_or_else(|| {
        let mut criteria = raw.criteria.keys().cloned().collect::<Vec<_>>();
        criteria.sort();
        criteria
            .into_iter()
            .map(|criterion| vec![criterion])
            .collect()
    });
    for requirement in &requirements {
        if requirement.is_empty() {
            return Err("advancement requirements may not contain an empty group".to_string());
        }
        for criterion in requirement {
            if !raw.criteria.contains_key(criterion) {
                return Err(format!(
                    "advancement requirement references unknown criterion {criterion:?}"
                ));
            }
        }
    }

    let display = raw
        .display
        .map(|display| {
            Ok::<_, String>(ClientAdvancementDisplay {
                title: display.title,
                description: display.description,
                item_icon: parse_item_stack(&display.icon)?,
                frame_type: match display.frame {
                    RawFrame::Task => FrameType::Task,
                    RawFrame::Challenge => FrameType::Challenge,
                    RawFrame::Goal => FrameType::Goal,
                },
                background_texture: display.background,
                show_toast: display.show_toast,
                hidden: display.hidden,
                x: 0.0,
                y: 0.0,
            })
        })
        .transpose()?;

    Ok(ClientAdvancement {
        id: id.clone(),
        parent,
        display,
        requirements,
        send_telemetry: raw.send_telemetry,
    })
}

pub(super) fn layout_and_sort(
    advancements: &FxHashMap<Identifier, ClientAdvancement>,
) -> Vec<ClientAdvancement> {
    let mut ordered = advancements.values().cloned().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        root_id(&left.id, advancements)
            .to_string()
            .cmp(&root_id(&right.id, advancements).to_string())
            .then_with(|| depth(&left.id, advancements).cmp(&depth(&right.id, advancements)))
            .then_with(|| left.id.to_string().cmp(&right.id.to_string()))
    });

    let mut next_row = FxHashMap::<Identifier, usize>::default();
    for advancement in &mut ordered {
        let root = root_id(&advancement.id, advancements);
        let row = next_row.entry(root).or_default();
        if let Some(display) = &mut advancement.display {
            display.x = depth(&advancement.id, advancements) as f32;
            display.y = *row as f32;
            *row += 1;
        }
    }
    ordered
}

fn root_id(id: &Identifier, advancements: &FxHashMap<Identifier, ClientAdvancement>) -> Identifier {
    let mut current = id.clone();
    let mut remaining = advancements.len();
    while remaining > 0 {
        let Some(parent) = advancements
            .get(&current)
            .and_then(|advancement| advancement.parent.clone())
        else {
            break;
        };
        if !advancements.contains_key(&parent) {
            break;
        }
        current = parent;
        remaining -= 1;
    }
    current
}

fn depth(id: &Identifier, advancements: &FxHashMap<Identifier, ClientAdvancement>) -> usize {
    let mut current = id.clone();
    let mut depth = 0;
    let mut remaining = advancements.len();
    while remaining > 0 {
        let Some(parent) = advancements
            .get(&current)
            .and_then(|advancement| advancement.parent.clone())
        else {
            break;
        };
        if !advancements.contains_key(&parent) {
            break;
        }
        current = parent;
        depth += 1;
        remaining -= 1;
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_matcha_advancement_display() {
        let advancement = parse_advancement(
            &Identifier::parse_static("main:tutorial/light_campfire"),
            br#"{
                "parent":"main:tutorial/root",
                "criteria":{"light":{"trigger":"minecraft:item_used_on_block"}},
                "display":{
                    "description":{"text":"Light Kindling"},
                    "icon":{"id":"minecraft:campfire"},
                    "title":{"text":"Prometheus be Damned"}
                }
            }"#,
        )
        .unwrap();
        assert_eq!(
            advancement.parent,
            Some(Identifier::parse_static("main:tutorial/root"))
        );
        assert!(advancement.display.is_some());
        assert_eq!(advancement.requirements, vec![vec!["light".to_string()]]);
    }
}
