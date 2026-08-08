use crate::world::loot::LootContextParameters;
use pumpkin_data::{Enchantment, item::Item, item_stack::ItemStack};
use rand::RngExt;
use serde_json::{Map, Value};

pub struct BlockLootTable {
    pools: Vec<LootPool>,
}

struct LootPool {
    rolls: f32,
    bonus_rolls: f32,
    entries: Vec<LootEntry>,
    conditions: Vec<LootCondition>,
}

struct LootEntry {
    content: LootEntryContent,
    weight: i32,
    quality: i32,
    conditions: Vec<LootCondition>,
}

enum LootEntryContent {
    Item(&'static Item),
    Alternatives(Vec<LootEntry>),
}

enum LootCondition {
    Inverted(Box<Self>),
    MatchToolEnchantments(Vec<EnchantmentRequirement>),
}

struct EnchantmentRequirement {
    enchantments: Vec<&'static Enchantment>,
    min_level: i32,
    max_level: Option<i32>,
}

pub(super) fn parse_block_loot_table(contents: &[u8]) -> Result<BlockLootTable, String> {
    let value: Value = serde_json::from_slice(contents)
        .map_err(|error| format!("loot table is not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "loot table root must be an object".to_string())?;
    reject_unknown_fields(object, &["type", "pools", "random_sequence"], "loot table")?;
    if object
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| kind != "minecraft:block")
    {
        return Err("loot table type must be minecraft:block".to_string());
    }
    let pools = object
        .get("pools")
        .and_then(Value::as_array)
        .ok_or_else(|| "block loot table is missing pools".to_string())?
        .iter()
        .map(parse_pool)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BlockLootTable { pools })
}

impl BlockLootTable {
    #[must_use]
    pub(crate) fn get_loot(&self, params: &LootContextParameters) -> Vec<ItemStack> {
        let mut stacks = Vec::new();
        let mut random = rand::rng();
        for pool in &self.pools {
            if !conditions_match(&pool.conditions, params) {
                continue;
            }
            let rolls = (pool.rolls + pool.bonus_rolls * params.luck)
                .floor()
                .max(0.0) as u32;
            for _ in 0..rolls {
                let valid_entries = pool
                    .entries
                    .iter()
                    .filter_map(|entry| {
                        if !conditions_match(&entry.conditions, params) {
                            return None;
                        }
                        let weight = (entry.weight as f32 + entry.quality as f32 * params.luck)
                            .floor()
                            .max(0.0) as i32;
                        (weight > 0).then_some((entry, weight))
                    })
                    .collect::<Vec<_>>();
                let total_weight = valid_entries.iter().map(|(_, weight)| weight).sum::<i32>();
                if total_weight == 0 {
                    continue;
                }
                let mut choice = random.random_range(0..total_weight);
                for (entry, weight) in valid_entries {
                    if choice < weight {
                        if let Some(mut loot) = entry.get_loot(params) {
                            stacks.append(&mut loot);
                        }
                        break;
                    }
                    choice -= weight;
                }
            }
        }
        stacks
    }
}

impl LootEntry {
    fn get_loot(&self, params: &LootContextParameters) -> Option<Vec<ItemStack>> {
        if !conditions_match(&self.conditions, params) {
            return None;
        }
        match &self.content {
            LootEntryContent::Item(item) => Some(vec![ItemStack::new(1, item)]),
            LootEntryContent::Alternatives(children) => {
                for child in children {
                    if let Some(loot) = child.get_loot(params) {
                        return Some(loot);
                    }
                }
                Some(Vec::new())
            }
        }
    }
}

impl LootCondition {
    fn matches(&self, params: &LootContextParameters) -> bool {
        match self {
            Self::Inverted(term) => !term.matches(params),
            Self::MatchToolEnchantments(requirements) => params.tool.as_ref().is_some_and(|tool| {
                requirements.iter().all(|requirement| {
                    requirement.enchantments.iter().any(|enchantment| {
                        let level = tool.get_enchantment_level(enchantment);
                        level >= requirement.min_level
                            && requirement.max_level.is_none_or(|max| level <= max)
                    })
                })
            }),
        }
    }
}

fn conditions_match(conditions: &[LootCondition], params: &LootContextParameters) -> bool {
    conditions.iter().all(|condition| condition.matches(params))
}

fn parse_pool(value: &Value) -> Result<LootPool, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "loot pool must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &["rolls", "bonus_rolls", "entries", "conditions", "functions"],
        "loot pool",
    )?;
    reject_functions(object, "loot pool")?;
    let rolls = parse_constant_number(
        object
            .get("rolls")
            .ok_or_else(|| "loot pool is missing rolls".to_string())?,
        "loot pool rolls",
    )?;
    let bonus_rolls = object.get("bonus_rolls").map_or(Ok(0.0), |value| {
        parse_constant_number(value, "loot pool bonus_rolls")
    })?;
    let entries = object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| "loot pool is missing entries".to_string())?
        .iter()
        .map(parse_entry)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(LootPool {
        rolls,
        bonus_rolls,
        entries,
        conditions: parse_conditions(object.get("conditions"))?,
    })
}

fn parse_entry(value: &Value) -> Result<LootEntry, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "loot entry must be an object".to_string())?;
    reject_unknown_fields(
        object,
        &[
            "type",
            "name",
            "children",
            "conditions",
            "functions",
            "weight",
            "quality",
        ],
        "loot entry",
    )?;
    reject_functions(object, "loot entry")?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "loot entry is missing type".to_string())?;
    let content = match kind {
        "minecraft:item" => {
            let raw_name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "item loot entry is missing name".to_string())?;
            let name = raw_name
                .strip_prefix("minecraft:")
                .ok_or_else(|| format!("loot item {raw_name} is not in the Minecraft registry"))?;
            let item = Item::from_registry_key(name)
                .ok_or_else(|| format!("loot item {raw_name} is not registered"))?;
            LootEntryContent::Item(item)
        }
        "minecraft:alternatives" => {
            let children = object
                .get("children")
                .and_then(Value::as_array)
                .ok_or_else(|| "alternatives loot entry is missing children".to_string())?
                .iter()
                .map(parse_entry)
                .collect::<Result<Vec<_>, _>>()?;
            LootEntryContent::Alternatives(children)
        }
        unsupported => {
            return Err(format!(
                "loot entry type {unsupported} is not supported yet"
            ));
        }
    };
    Ok(LootEntry {
        content,
        weight: parse_i32_field(object, "weight", 1)?,
        quality: parse_i32_field(object, "quality", 0)?,
        conditions: parse_conditions(object.get("conditions"))?,
    })
}

fn parse_conditions(value: Option<&Value>) -> Result<Vec<LootCondition>, String> {
    value.map_or(Ok(Vec::new()), |value| {
        value
            .as_array()
            .ok_or_else(|| "loot conditions must be a list".to_string())?
            .iter()
            .map(parse_condition)
            .collect()
    })
}

fn parse_condition(value: &Value) -> Result<LootCondition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "loot condition must be an object".to_string())?;
    let kind = object
        .get("condition")
        .and_then(Value::as_str)
        .ok_or_else(|| "loot condition is missing condition".to_string())?;
    match kind {
        "minecraft:inverted" => {
            reject_unknown_fields(object, &["condition", "term"], "inverted loot condition")?;
            Ok(LootCondition::Inverted(Box::new(parse_condition(
                object
                    .get("term")
                    .ok_or_else(|| "inverted loot condition is missing term".to_string())?,
            )?)))
        }
        "minecraft:match_tool" => {
            reject_unknown_fields(
                object,
                &["condition", "predicate"],
                "match_tool loot condition",
            )?;
            Ok(LootCondition::MatchToolEnchantments(
                parse_tool_enchantments(
                    object
                        .get("predicate")
                        .ok_or_else(|| "match_tool condition is missing predicate".to_string())?,
                )?,
            ))
        }
        unsupported => Err(format!("loot condition {unsupported} is not supported yet")),
    }
}

fn parse_tool_enchantments(value: &Value) -> Result<Vec<EnchantmentRequirement>, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "match_tool predicate must be an object".to_string())?;
    reject_unknown_fields(object, &["predicates"], "match_tool predicate")?;
    let predicates = object
        .get("predicates")
        .and_then(Value::as_object)
        .ok_or_else(|| "match_tool predicate is missing predicates".to_string())?;
    reject_unknown_fields(
        predicates,
        &["minecraft:enchantments"],
        "match_tool component predicates",
    )?;
    predicates
        .get("minecraft:enchantments")
        .and_then(Value::as_array)
        .ok_or_else(|| "match_tool predicate is missing enchantments".to_string())?
        .iter()
        .map(parse_enchantment_requirement)
        .collect()
}

fn parse_enchantment_requirement(value: &Value) -> Result<EnchantmentRequirement, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "enchantment predicate must be an object".to_string())?;
    reject_unknown_fields(object, &["enchantments", "levels"], "enchantment predicate")?;
    let raw_enchantments = object
        .get("enchantments")
        .ok_or_else(|| "enchantment predicate is missing enchantments".to_string())?;
    let names = if let Some(name) = raw_enchantments.as_str() {
        vec![name]
    } else {
        raw_enchantments
            .as_array()
            .ok_or_else(|| "enchantments must be a string or list".to_string())?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| "enchantment names must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let enchantments = names
        .into_iter()
        .map(|name| {
            Enchantment::from_name(name)
                .ok_or_else(|| format!("enchantment {name} is not registered"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if enchantments.is_empty() {
        return Err("enchantment predicate may not be empty".to_string());
    }

    let (min_level, max_level) = match object.get("levels") {
        None => (1, None),
        Some(Value::Number(level)) => {
            let level = level
                .as_i64()
                .and_then(|level| i32::try_from(level).ok())
                .ok_or_else(|| "enchantment level must be an integer".to_string())?;
            (level, Some(level))
        }
        Some(Value::Object(range)) => (
            parse_optional_i32(range.get("min"), "minimum enchantment level")?.unwrap_or(0),
            parse_optional_i32(range.get("max"), "maximum enchantment level")?,
        ),
        Some(_) => return Err("enchantment levels must be an integer or range".to_string()),
    };
    Ok(EnchantmentRequirement {
        enchantments,
        min_level,
        max_level,
    })
}

fn parse_optional_i32(value: Option<&Value>, label: &str) -> Result<Option<i32>, String> {
    value.map_or(Ok(None), |value| {
        value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| format!("{label} must be an integer"))
    })
}

fn parse_i32_field(object: &Map<String, Value>, field: &str, default: i32) -> Result<i32, String> {
    object.get(field).map_or(Ok(default), |value| {
        value
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| format!("loot entry {field} must be an integer"))
    })
}

fn parse_constant_number(value: &Value, label: &str) -> Result<f32, String> {
    value
        .as_f64()
        .map(|value| value as f32)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| format!("{label} must be a non-negative constant number"))
}

fn reject_functions(object: &Map<String, Value>, label: &str) -> Result<(), String> {
    if object.get("functions").is_some_and(|functions| {
        functions
            .as_array()
            .is_none_or(|functions| !functions.is_empty())
    }) {
        return Err(format!("{label} functions are not supported yet"));
    }
    Ok(())
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
    label: &str,
) -> Result<(), String> {
    if let Some(field) = object
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(format!("{label} field {field} is not supported yet"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MATCHA_GRAVEL: &[u8] = br#"{
      "type":"minecraft:block",
      "pools":[
        {"rolls":1,"bonus_rolls":0,"entries":[{"type":"minecraft:alternatives","children":[{"type":"minecraft:item","name":"minecraft:gravel","conditions":[{"condition":"minecraft:match_tool","predicate":{"predicates":{"minecraft:enchantments":[{"enchantments":"minecraft:silk_touch","levels":{"min":1}}]}}}]}]}]},
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:dirt","conditions":[{"condition":"minecraft:inverted","term":{"condition":"minecraft:match_tool","predicate":{"predicates":{"minecraft:enchantments":[{"enchantments":"minecraft:silk_touch","levels":{"min":1}}]}}}}]}]},
        {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:flint","conditions":[{"condition":"minecraft:inverted","term":{"condition":"minecraft:match_tool","predicate":{"predicates":{"minecraft:enchantments":[{"enchantments":"minecraft:silk_touch","levels":{"min":1}}]}}}}]}]}
      ]
    }"#;

    #[test]
    fn matcha_gravel_drops_dirt_and_flint_without_silk_touch() {
        let table = parse_block_loot_table(MATCHA_GRAVEL).unwrap();
        let loot = table.get_loot(&LootContextParameters::default());
        assert_eq!(
            loot.iter()
                .map(|stack| stack.item.registry_key)
                .collect::<Vec<_>>(),
            ["dirt", "flint"]
        );
    }

    #[test]
    fn matcha_gravel_drops_gravel_with_silk_touch() {
        let table = parse_block_loot_table(MATCHA_GRAVEL).unwrap();
        let mut tool = ItemStack::new(1, &Item::IRON_SHOVEL);
        tool.add_enchantment(&Enchantment::SILK_TOUCH, 1);
        let loot = table.get_loot(&LootContextParameters {
            tool: Some(tool),
            ..Default::default()
        });
        assert_eq!(
            loot.iter()
                .map(|stack| stack.item.registry_key)
                .collect::<Vec<_>>(),
            ["gravel"]
        );
    }
}
