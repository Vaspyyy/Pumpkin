use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::snbt::SnbtParser;
use crate::command::string_reader::StringReader;
use pumpkin_data::data_component::DataComponent;
use pumpkin_data::data_component_impl::{DataComponentImpl, read_data};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_data::tag::{RegistryKey, get_tag_ids};
use pumpkin_data::translation;
use pumpkin_util::text::TextComponent;

const INVALID_ITEM_ERROR: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENT_ITEM_ID_INVALID,
    translation::java::ARGUMENT_ITEM_ID_INVALID,
);
const UNKNOWN_COMPONENT_ERROR: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_ITEM_COMPONENT_UNKNOWN,
    translation::java::ARGUMENTS_ITEM_COMPONENT_UNKNOWN,
);
const MALFORMED_COMPONENT_ERROR: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_ITEM_COMPONENT_MALFORMED,
    translation::java::ARGUMENTS_ITEM_COMPONENT_MALFORMED,
);
const UNKNOWN_TAG_ERROR: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_ITEM_TAG_UNKNOWN,
    translation::java::ARGUMENTS_ITEM_TAG_UNKNOWN,
);

enum ItemChoice {
    Any,
    Item(&'static Item),
    Tag(Vec<u16>),
}

struct ComponentPredicate {
    component: DataComponent,
    expected: Option<Box<dyn DataComponentImpl>>,
    negated: bool,
}

pub struct ItemPredicate {
    item: ItemChoice,
    components: Vec<ComponentPredicate>,
}

impl ItemPredicate {
    #[must_use]
    pub fn test(&self, stack: &ItemStack) -> bool {
        if stack.item_count == 0 {
            return false;
        }

        let item_matches = match &self.item {
            ItemChoice::Any => true,
            ItemChoice::Item(item) => stack.get_item() == *item,
            ItemChoice::Tag(ids) => ids.contains(&stack.get_item().id),
        };
        item_matches
            && self.components.iter().all(|predicate| {
                let actual = stack.get_data_component_dyn(predicate.component);
                let matches = predicate.expected.as_ref().map_or_else(
                    || actual.is_some(),
                    |expected| actual.is_some_and(|actual| expected.equal(actual)),
                );
                matches != predicate.negated
            })
    }
}

pub struct ItemPredicateArgumentType;

impl ArgumentType for ItemPredicateArgumentType {
    type Item = ItemPredicate;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let start = reader.cursor();
        read_item_predicate_token(reader);
        let raw = &reader.string()[start..reader.cursor()];
        let (choice, component_text) = split_choice_and_components(raw);

        let item = if choice == "*" {
            ItemChoice::Any
        } else if let Some(tag) = choice.strip_prefix('#') {
            let normalized = normalize_identifier(tag);
            let ids = get_tag_ids(RegistryKey::Item, &normalized).ok_or_else(|| {
                UNKNOWN_TAG_ERROR.create(reader, TextComponent::text(normalized.clone()))
            })?;
            ItemChoice::Tag(ids.to_vec())
        } else {
            let normalized = normalize_identifier(choice);
            let item = Item::from_registry_key(&normalized).ok_or_else(|| {
                INVALID_ITEM_ERROR.create(reader, TextComponent::text(normalized.clone()))
            })?;
            ItemChoice::Item(item)
        };

        let components = component_text
            .map(|components| parse_components(components, reader))
            .transpose()?
            .unwrap_or_default();
        Ok(ItemPredicate { item, components })
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::ItemPredicate
    }

    fn examples(&self) -> Vec<String> {
        examples!(
            "minecraft:stick",
            "#minecraft:planks",
            "*[minecraft:repair_cost]"
        )
    }
}

impl ItemPredicateArgumentType {
    pub fn get<'a>(
        context: &'a CommandContext,
        name: &str,
    ) -> Result<&'a ItemPredicate, CommandSyntaxError> {
        context.get_argument::<ItemPredicate>(name)
    }
}

fn read_item_predicate_token(reader: &mut StringReader) {
    let mut square_depth = 0usize;
    let mut curly_depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    while let Some(c) = reader.peek() {
        if quote.is_none() && square_depth == 0 && curly_depth == 0 && c.is_whitespace() {
            break;
        }
        reader.skip();
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if c == delimiter {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            '{' => curly_depth += 1,
            '}' => curly_depth = curly_depth.saturating_sub(1),
            _ => {}
        }
    }
}

fn split_choice_and_components(raw: &str) -> (&str, Option<&str>) {
    raw.find('[').map_or((raw, None), |start| {
        let components = raw.strip_suffix(']').map(|trimmed| &trimmed[start + 1..]);
        (&raw[..start], components)
    })
}

fn parse_components(
    raw: &str,
    reader: &StringReader,
) -> Result<Vec<ComponentPredicate>, CommandSyntaxError> {
    split_top_level(raw, ',')
        .into_iter()
        .map(|raw_component| {
            let raw_component = raw_component.trim();
            if raw_component.is_empty() {
                return Err(
                    MALFORMED_COMPONENT_ERROR.create(reader, TextComponent::text(raw.to_string()))
                );
            }
            let (negated, raw_component) = raw_component
                .strip_prefix('!')
                .map_or((false, raw_component), |component| (true, component));
            let (name, raw_expected) = split_once_top_level(raw_component, '=');
            let normalized = normalize_identifier(name.trim());
            let component = DataComponent::try_from_name(&normalized).ok_or_else(|| {
                UNKNOWN_COMPONENT_ERROR.create(reader, TextComponent::text(normalized.clone()))
            })?;
            let expected = raw_expected
                .map(|raw_expected| {
                    let mut component_reader = StringReader::new(raw_expected.trim());
                    let tag = SnbtParser::parse_for_commands(&mut component_reader)?;
                    if component_reader.can_read_char() {
                        return Err(MALFORMED_COMPONENT_ERROR
                            .create(reader, TextComponent::text(raw_component.to_string())));
                    }
                    read_data(component, &tag).ok_or_else(|| {
                        MALFORMED_COMPONENT_ERROR
                            .create(reader, TextComponent::text(raw_component.to_string()))
                    })
                })
                .transpose()?;
            Ok(ComponentPredicate {
                component,
                expected,
                negated,
            })
        })
        .collect()
}

fn normalize_identifier(raw: &str) -> String {
    if raw.contains(':') {
        raw.to_string()
    } else {
        format!("minecraft:{raw}")
    }
}

fn split_once_top_level(raw: &str, delimiter: char) -> (&str, Option<&str>) {
    find_top_level(raw, delimiter).map_or((raw, None), |index| {
        (&raw[..index], Some(&raw[index + 1..]))
    })
}

fn split_top_level(raw: &str, delimiter: char) -> Vec<&str> {
    let mut values = Vec::new();
    let mut start = 0;
    for index in top_level_indices(raw, delimiter) {
        values.push(&raw[start..index]);
        start = index + delimiter.len_utf8();
    }
    values.push(&raw[start..]);
    values
}

fn find_top_level(raw: &str, delimiter: char) -> Option<usize> {
    top_level_indices(raw, delimiter).into_iter().next()
}

fn top_level_indices(raw: &str, delimiter: char) -> Vec<usize> {
    let mut square_depth = 0usize;
    let mut curly_depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut indices = Vec::new();
    for (index, c) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && quote.is_some() {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if c == delimiter {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '[' => square_depth += 1,
            ']' => square_depth = square_depth.saturating_sub(1),
            '{' => curly_depth += 1,
            '}' => curly_depth = curly_depth.saturating_sub(1),
            _ if c == delimiter && square_depth == 0 && curly_depth == 0 => indices.push(index),
            _ => {}
        }
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::data_component_impl::{ItemModelImpl, MaxStackSizeImpl, PotionContentsImpl};

    fn parse(raw: &str) -> ItemPredicate {
        ItemPredicateArgumentType
            .parse(&mut StringReader::new(raw))
            .unwrap()
    }

    #[test]
    fn matches_plain_item_and_rejects_empty_stack() {
        let predicate = parse("minecraft:blaze_powder");
        assert!(predicate.test(&ItemStack::new(1, &Item::BLAZE_POWDER)));
        assert!(!predicate.test(ItemStack::EMPTY));
        assert!(!predicate.test(&ItemStack::new(1, &Item::STICK)));
    }

    #[test]
    fn matches_presence_component() {
        let predicate = parse("*[minecraft:repair_cost]");
        assert!(predicate.test(&ItemStack::new(1, &Item::DIAMOND_PICKAXE)));
    }

    #[test]
    fn matches_item_model_component() {
        let predicate = parse("*[minecraft:item_model=\"minecraft:heart_container\"]");
        let stack = ItemStack::new_with_component(
            1,
            &Item::PAPER,
            vec![(
                DataComponent::ItemModel,
                Some(Box::new(ItemModelImpl {
                    id: "minecraft:heart_container".to_string().into(),
                })),
            )],
        );
        assert!(predicate.test(&stack));
    }

    #[test]
    fn matches_potion_and_negated_max_stack_size_components() {
        let predicate = parse(
            "minecraft:potion[minecraft:potion_contents={potion:\"minecraft:water\"},!minecraft:max_stack_size=64]",
        );
        let stack = ItemStack::new_with_component(
            1,
            &Item::POTION,
            vec![
                (
                    DataComponent::PotionContents,
                    Some(Box::new(PotionContentsImpl {
                        potion_id: pumpkin_data::potion::Potion::from_name("water")
                            .map(|potion| i32::from(potion.id)),
                        custom_color: None,
                        custom_effects: Vec::new(),
                        custom_name: None,
                    })),
                ),
                (
                    DataComponent::MaxStackSize,
                    Some(Box::new(MaxStackSizeImpl { size: 1 })),
                ),
            ],
        );
        assert!(predicate.test(&stack));
    }
}
