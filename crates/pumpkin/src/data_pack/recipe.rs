use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::read_data;
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::recipes::RecipeCategoryTypes;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::codec::recipe::{
    DynamicRecipe, OwnedCraftingRecipe, OwnedRecipeIngredient, OwnedRecipeResult,
};
use pumpkin_util::identifier::Identifier;
use serde_json::Value;

pub(super) fn parse_recipe(id: &Identifier, contents: &[u8]) -> Result<DynamicRecipe, String> {
    let value: Value = serde_json::from_slice(contents)
        .map_err(|error| format!("recipe is not valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "recipe root must be an object".to_string())?;
    let recipe_type = string_field(object, "type")?;
    let category = parse_category(object.get("category").and_then(Value::as_str));
    let group = object
        .get("group")
        .and_then(Value::as_str)
        .filter(|group| !group.is_empty())
        .map(str::to_owned);
    match recipe_type {
        "minecraft:crafting_shaped" => {
            let result = parse_recipe_result(object)?;
            let pattern = object
                .get("pattern")
                .and_then(Value::as_array)
                .ok_or_else(|| "shaped recipe is missing pattern".to_string())?
                .iter()
                .map(|row| {
                    row.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "shaped recipe pattern rows must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let pattern = normalize_pattern(pattern)?;
            let raw_key = object
                .get("key")
                .and_then(Value::as_object)
                .ok_or_else(|| "shaped recipe is missing key".to_string())?;
            let mut key = Vec::with_capacity(raw_key.len());
            for (symbol, ingredient) in raw_key {
                let mut chars = symbol.chars();
                let symbol = chars
                    .next()
                    .filter(|_| chars.next().is_none())
                    .ok_or_else(|| "shaped recipe keys must be one character".to_string())?;
                if symbol == ' ' {
                    return Err("shaped recipe key may not define a space".to_string());
                }
                key.push((symbol, parse_ingredient(ingredient)?));
            }
            for symbol in pattern.iter().flat_map(|row| row.chars()) {
                if symbol != ' ' && !key.iter().any(|(key, _)| *key == symbol) {
                    return Err(format!("pattern references undefined key {symbol:?}"));
                }
            }
            Ok(DynamicRecipe::Crafting(OwnedCraftingRecipe::Shaped {
                recipe_id: id.to_string(),
                category,
                group,
                show_notification: object
                    .get("show_notification")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                key,
                pattern,
                result,
            }))
        }
        "minecraft:crafting_shapeless" => {
            let result = parse_recipe_result(object)?;
            let ingredients = object
                .get("ingredients")
                .and_then(Value::as_array)
                .ok_or_else(|| "shapeless recipe is missing ingredients".to_string())?
                .iter()
                .map(parse_ingredient)
                .collect::<Result<Vec<_>, _>>()?;
            if ingredients.is_empty() || ingredients.len() > 9 {
                return Err("shapeless recipes must contain 1 to 9 ingredients".to_string());
            }
            Ok(DynamicRecipe::Crafting(OwnedCraftingRecipe::Shapeless {
                recipe_id: id.to_string(),
                category,
                group,
                ingredients,
                result,
            }))
        }
        unsupported => Err(format!("recipe type {unsupported} is not supported yet")),
    }
}

fn parse_recipe_result(
    object: &serde_json::Map<String, Value>,
) -> Result<OwnedRecipeResult, String> {
    Ok(OwnedRecipeResult {
        item_stack: parse_item_stack(
            object
                .get("result")
                .ok_or_else(|| "recipe is missing result".to_string())?,
        )?,
    })
}

fn string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("recipe is missing string field {key}"))
}

fn parse_category(category: Option<&str>) -> RecipeCategoryTypes {
    match category {
        Some("equipment") => RecipeCategoryTypes::Equipment,
        Some("building") => RecipeCategoryTypes::Building,
        Some("redstone") => RecipeCategoryTypes::Restone,
        Some("food") => RecipeCategoryTypes::Food,
        Some("blocks") => RecipeCategoryTypes::Blocks,
        _ => RecipeCategoryTypes::Misc,
    }
}

fn parse_ingredient(value: &Value) -> Result<OwnedRecipeIngredient, String> {
    if let Some(id) = value.as_str() {
        return Ok(id.strip_prefix('#').map_or_else(
            || OwnedRecipeIngredient::Simple(id.to_string()),
            |tag| OwnedRecipeIngredient::Tagged(tag.to_string()),
        ));
    }
    if let Some(ids) = value.as_array() {
        let ids = ids
            .iter()
            .map(|id| {
                id.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "ingredient alternatives must be strings".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        if ids.is_empty() {
            return Err("ingredient alternatives may not be empty".to_string());
        }
        return Ok(OwnedRecipeIngredient::OneOf(ids));
    }
    Err("ingredient must be an item/tag string or a list of item strings".to_string())
}

pub(super) fn parse_item_stack(value: &Value) -> Result<ItemStack, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "recipe result must be an object".to_string())?;
    let raw_id = string_field(object, "id")?;
    let id = raw_id.strip_prefix("minecraft:").unwrap_or(raw_id);
    let item = Item::from_registry_key(id)
        .ok_or_else(|| format!("recipe result item {raw_id} is not registered"))?;
    let count = object
        .get("count")
        .and_then(Value::as_u64)
        .and_then(|count| u8::try_from(count).ok())
        .unwrap_or(1);
    if count == 0 {
        return Err("recipe result count must be positive".to_string());
    }

    let mut components = Vec::new();
    if let Some(raw_components) = object.get("components").and_then(Value::as_object) {
        for (raw_name, value) in raw_components {
            let (removed, name) = raw_name
                .strip_prefix('!')
                .map_or((false, raw_name.as_str()), |name| (true, name));
            let Some(component) = DataComponent::try_from_name(name) else {
                continue;
            };
            if removed {
                components.push((component, None));
            } else if let Some(data) = read_data(component, &json_to_nbt(value)) {
                components.push((component, Some(data)));
            }
        }
    }

    Ok(ItemStack::new_with_component(count, item, components))
}

fn json_to_nbt(value: &Value) -> NbtTag {
    match value {
        Value::Null => NbtTag::End,
        Value::Bool(value) => NbtTag::Byte(i8::from(*value)),
        Value::Number(value) => value.as_i64().map_or_else(
            || NbtTag::Float(value.as_f64().unwrap_or_default() as f32),
            |value| i32::try_from(value).map_or(NbtTag::Long(value), NbtTag::Int),
        ),
        Value::String(value) => NbtTag::String(value.clone().into_boxed_str()),
        Value::Array(values) => NbtTag::List(values.iter().map(json_to_nbt).collect()),
        Value::Object(values) => {
            let mut compound = NbtCompound::new();
            for (key, value) in values {
                compound.put(key, json_to_nbt(value));
            }
            NbtTag::Compound(compound)
        }
    }
}

fn normalize_pattern(mut pattern: Vec<String>) -> Result<Vec<String>, String> {
    let Some(width) = pattern.iter().map(|row| row.chars().count()).max() else {
        return Err("shaped recipe pattern may not be empty".to_string());
    };
    if width == 0 || width > 3 || pattern.len() > 3 {
        return Err("shaped recipe pattern must fit a 3x3 crafting grid".to_string());
    }
    for row in &mut pattern {
        row.extend(std::iter::repeat_n(' ', width - row.chars().count()));
    }
    Ok(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::data_component_impl::BlockStateImpl;

    #[test]
    fn parses_matcha_kindling_recipe_with_unlit_component() {
        let recipe = parse_recipe(
            &Identifier::parse_static("crafting:campfire"),
            br##"{
                "type":"minecraft:crafting_shaped",
                "group":"campfire",
                "category":"misc",
                "key":{"L":"#minecraft:logs","S":"minecraft:stick"},
                "pattern":["SS","LL"],
                "result":{"components":{"minecraft:block_state":{"lit":"false"}},"count":1,"id":"minecraft:campfire"}
            }"##,
        )
        .unwrap();
        let DynamicRecipe::Crafting(OwnedCraftingRecipe::Shaped { result, .. }) = recipe else {
            panic!("expected shaped recipe");
        };
        let state = result
            .item_stack
            .get_data_component::<BlockStateImpl>()
            .expect("block state component should be retained");
        assert_eq!(state.properties[0].0, "lit");
        assert_eq!(state.properties[0].1, "false");
    }

    #[test]
    fn normalizes_matcha_unicode_and_ragged_patterns() {
        let unicode = normalize_pattern(vec!["££".to_string(), "££".to_string()]).unwrap();
        assert_eq!(unicode, ["££", "££"]);

        let ragged = normalize_pattern(vec![" ~".to_string(), "O".to_string()]).unwrap();
        assert_eq!(ragged, [" ~", "O "]);
    }
}
