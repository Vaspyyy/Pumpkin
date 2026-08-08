use std::pin::Pin;

use crate::command::{
    argument_types::{
        argument_type::{ArgumentType, JavaClientArgumentType},
        entity::{EntityArgumentType, NO_ENTITIES_ERROR_TYPE, NOT_SINGLE_ENTITY_ERROR_TYPE},
        entity_selector::EntitySelector,
    },
    context::{command_context::CommandContext, command_source::CommandSource},
    errors::command_syntax_error::CommandSyntaxError,
    string_reader::StringReader,
    suggestion::suggestions::{Suggestions, SuggestionsBuilder},
};

pub enum ScoreHolder {
    Name(String),
    Selector(Box<EntitySelector>),
    Wildcard,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ScoreHolderArgumentType {
    Single,
    Multiple,
}

impl ScoreHolderArgumentType {
    const fn allows_multiple(self) -> bool {
        matches!(self, Self::Multiple)
    }

    const fn entity_argument(self) -> EntityArgumentType {
        if self.allows_multiple() {
            EntityArgumentType::Entities
        } else {
            EntityArgumentType::Entity
        }
    }

    fn parse_literal(reader: &mut StringReader) -> ScoreHolder {
        let start = reader.cursor();
        reader.read_until_space();
        let name = reader.string()[start..reader.cursor()].to_string();
        if name == "*" {
            ScoreHolder::Wildcard
        } else {
            ScoreHolder::Name(name)
        }
    }

    pub async fn get_score_holders(
        context: &CommandContext<'_>,
        name: &str,
    ) -> Result<Vec<String>, CommandSyntaxError> {
        match context.get_argument::<ScoreHolder>(name)? {
            ScoreHolder::Name(name) => Ok(vec![name.clone()]),
            ScoreHolder::Wildcard => {
                let scoreboard = context.world().scoreboard.lock().await;
                Ok(scoreboard
                    .get_tracked_players()
                    .into_iter()
                    .map(ToString::to_string)
                    .collect())
            }
            ScoreHolder::Selector(selector) => {
                let entities = selector.find_entities(context.source.as_ref()).await?;
                if entities.is_empty() {
                    return Err(NO_ENTITIES_ERROR_TYPE.create_without_context());
                }
                Ok(entities
                    .into_iter()
                    .map(|entity| {
                        entity.get_player().map_or_else(
                            || entity.get_entity().entity_uuid.to_string(),
                            |player| player.gameprofile.name.clone(),
                        )
                    })
                    .collect())
            }
        }
    }

    pub async fn get_score_holder(
        context: &CommandContext<'_>,
        name: &str,
    ) -> Result<String, CommandSyntaxError> {
        let mut holders = Self::get_score_holders(context, name).await?;
        match holders.len() {
            0 => Err(NO_ENTITIES_ERROR_TYPE.create_without_context()),
            1 => Ok(holders.pop().expect("score holder length was checked")),
            _ => Err(NOT_SINGLE_ENTITY_ERROR_TYPE.create_without_context()),
        }
    }
}

impl ArgumentType for ScoreHolderArgumentType {
    type Item = ScoreHolder;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        if reader.peek() == Some('@') {
            self.entity_argument()
                .parse(reader)
                .map(Box::new)
                .map(ScoreHolder::Selector)
        } else {
            Ok(Self::parse_literal(reader))
        }
    }

    fn parse_with_source<'a>(
        &'a self,
        reader: &'a mut StringReader,
        source: &'a CommandSource,
    ) -> Pin<Box<dyn Future<Output = Result<Self::Item, CommandSyntaxError>> + Send + 'a>> {
        Box::pin(async move {
            if reader.peek() == Some('@') {
                self.entity_argument()
                    .parse_with_source(reader, source)
                    .await
                    .map(Box::new)
                    .map(ScoreHolder::Selector)
            } else {
                Ok(Self::parse_literal(reader))
            }
        })
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::ScoreHolder {
            flags: u8::from(self.allows_multiple())
                * JavaClientArgumentType::SCORE_HOLDER_FLAG_ALLOW_MULTIPLE,
        }
    }

    fn list_suggestions<'a>(
        &'a self,
        context: &'a CommandContext,
        mut builder: SuggestionsBuilder,
    ) -> Pin<Box<dyn Future<Output = Suggestions> + Send + 'a>> {
        Box::pin(async move {
            let scoreboard = context.world().scoreboard.lock().await;
            for holder in scoreboard.get_tracked_players() {
                builder = builder.filter_and_suggest_one(holder);
            }
            if self.allows_multiple() {
                builder = builder.filter_and_suggest_one("*");
            }
            builder.build()
        })
    }

    fn examples(&self) -> Vec<String> {
        examples!("Player", "0123", "@e", "#constant")
    }
}

#[cfg(test)]
mod tests {
    use super::{ScoreHolder, ScoreHolderArgumentType};
    use crate::command::{
        argument_types::argument_type::ArgumentType, string_reader::StringReader,
    };

    #[test]
    fn parses_fake_holder_names_and_wildcard() {
        let mut fake = StringReader::new("#constant");
        assert!(matches!(
            ScoreHolderArgumentType::Multiple.parse(&mut fake),
            Ok(ScoreHolder::Name(name)) if name == "#constant"
        ));

        let mut wildcard = StringReader::new("*");
        assert!(matches!(
            ScoreHolderArgumentType::Multiple.parse(&mut wildcard),
            Ok(ScoreHolder::Wildcard)
        ));
    }
}
