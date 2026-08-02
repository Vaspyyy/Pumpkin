use crate::command::argument_types::FromStringReader;
use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::string_reader::StringReader;
use pumpkin_util::identifier::Identifier;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionReference {
    pub id: Identifier,
    pub is_tag: bool,
}

pub struct FunctionArgumentType;

impl ArgumentType for FunctionArgumentType {
    type Item = FunctionReference;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let is_tag = reader.peek() == Some('#');
        if is_tag {
            reader.skip();
        }
        Ok(FunctionReference {
            id: Identifier::from_reader(reader)?,
            is_tag,
        })
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::Function
    }

    fn examples(&self) -> Vec<String> {
        examples!("foo", "foo:bar", "#foo:bar")
    }
}

impl FunctionArgumentType {
    pub fn get<'a>(
        context: &'a crate::command::context::command_context::CommandContext,
        name: &str,
    ) -> Result<&'a FunctionReference, CommandSyntaxError> {
        context.get_argument(name)
    }
}

#[cfg(test)]
mod tests {
    use super::{FunctionArgumentType, FunctionReference};
    use crate::command::argument_types::argument_type::ArgumentType;
    use crate::command::string_reader::StringReader;
    use pumpkin_util::identifier::Identifier;

    #[test]
    fn parses_functions_and_tags() {
        let mut function = StringReader::new("example:run");
        assert_eq!(
            FunctionArgumentType.parse(&mut function).unwrap(),
            FunctionReference {
                id: Identifier::parse("example:run").unwrap(),
                is_tag: false,
            }
        );

        let mut tag = StringReader::new("#example:group");
        assert_eq!(
            FunctionArgumentType.parse(&mut tag).unwrap(),
            FunctionReference {
                id: Identifier::parse("example:group").unwrap(),
                is_tag: true,
            }
        );
    }
}
