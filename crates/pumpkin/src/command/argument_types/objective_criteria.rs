use crate::command::{
    argument_types::argument_type::{ArgumentType, JavaClientArgumentType},
    errors::command_syntax_error::CommandSyntaxError,
    string_reader::StringReader,
};

/// Parses an objective criterion, whose modern statistic form contains a colon.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct ObjectiveCriteriaArgumentType;

impl ArgumentType for ObjectiveCriteriaArgumentType {
    type Item = String;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let start = reader.cursor();
        reader.read_until_space();
        Ok(reader.string()[start..reader.cursor()].to_string())
    }

    fn client_side_parser(&self) -> JavaClientArgumentType {
        JavaClientArgumentType::ObjectiveCriteria
    }

    fn examples(&self) -> Vec<String> {
        examples!("dummy", "trigger", "minecraft.custom:minecraft.sneak_time")
    }
}

#[cfg(test)]
mod tests {
    use super::ObjectiveCriteriaArgumentType;
    use crate::command::{
        argument_types::argument_type::ArgumentType, string_reader::StringReader,
    };

    #[test]
    fn parses_modern_statistic_criterion() {
        let mut reader = StringReader::new("minecraft.custom:minecraft.sneak_time trailing");
        assert_eq!(
            ObjectiveCriteriaArgumentType
                .parse(&mut reader)
                .expect("valid criterion"),
            "minecraft.custom:minecraft.sneak_time"
        );
    }
}
