use crate::command::argument_types::FromStringReader;
use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::argument_types::resource_key::BIOME_REGISTRY;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::string_reader::StringReader;
use crate::data_pack::DataPackManager;
use pumpkin_data::biome::Biome;
use pumpkin_data::translation;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::text::TextComponent;

const INVALID_BIOME_ERROR: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENT_RESOURCE_INVALID_TYPE,
    translation::java::ARGUMENT_RESOURCE_INVALID_TYPE,
);
pub enum BiomePredicate {
    Biome(&'static Biome),
    Tag(Identifier),
}

impl BiomePredicate {
    #[must_use]
    pub fn test(&self, biome: &Biome, data_packs: &DataPackManager) -> bool {
        match self {
            Self::Biome(expected) => expected.id == biome.id,
            Self::Tag(id) => data_packs.biome_is_tagged(id, biome),
        }
    }
}

pub struct BiomePredicateArgumentType;

impl ArgumentType for BiomePredicateArgumentType {
    type Item = BiomePredicate;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let is_tag = reader.peek() == Some('#');
        if is_tag {
            reader.skip();
        }
        let id = Identifier::from_reader(reader)?;
        if is_tag {
            Ok(BiomePredicate::Tag(id))
        } else {
            let biome = Biome::from_name(id.path()).ok_or_else(|| {
                INVALID_BIOME_ERROR.create(reader, TextComponent::text(id.to_string()))
            })?;
            Ok(BiomePredicate::Biome(biome))
        }
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::ResourceOrTag {
            identifier: BIOME_REGISTRY.clone(),
        }
    }

    fn examples(&self) -> Vec<String> {
        examples!("minecraft:plains", "#minecraft:is_frozen")
    }
}

impl BiomePredicateArgumentType {
    pub fn get<'a>(
        context: &'a CommandContext,
        name: &str,
    ) -> Result<&'a BiomePredicate, CommandSyntaxError> {
        context.get_argument::<BiomePredicate>(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_biome_and_tag() {
        let plains = BiomePredicateArgumentType
            .parse(&mut StringReader::new("minecraft:plains"))
            .unwrap();
        let data_packs = DataPackManager::default();
        assert!(plains.test(&Biome::PLAINS, &data_packs));

        let frozen = BiomePredicateArgumentType
            .parse(&mut StringReader::new("#minecraft:is_overworld"))
            .unwrap();
        assert!(frozen.test(&Biome::FROZEN_RIVER, &data_packs));
        assert!(!frozen.test(&Biome::THE_END, &data_packs));
    }
}
