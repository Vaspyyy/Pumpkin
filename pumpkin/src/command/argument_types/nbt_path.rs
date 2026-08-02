use crate::command::argument_types::argument_type::{ArgumentType, JavaClientArgumentType};
use crate::command::context::command_context::CommandContext;
use crate::command::errors::command_syntax_error::CommandSyntaxError;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::string_reader::StringReader;
use pumpkin_data::translation;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;

const INVALID_NODE_ERROR: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_NBTPATH_NODE_INVALID,
    translation::java::ARGUMENTS_NBTPATH_NODE_INVALID,
);

#[derive(Clone, Debug, PartialEq, Eq)]
enum PathNode {
    Key(String),
    Index(i32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NbtPath(Vec<PathNode>);

impl NbtPath {
    /// Replaces the value selected by this path. Missing final compound keys
    /// are created, while all parent nodes must already exist.
    pub fn set(&self, root: &mut NbtCompound, value: NbtTag) -> bool {
        let Some(PathNode::Key(key)) = self.0.first() else {
            return false;
        };
        if self.0.len() == 1 {
            root.child_tags.insert(key.clone().into(), value);
            return true;
        }
        let Some(current) = root.child_tags.get_mut(key.as_str()) else {
            return false;
        };
        set_tag(current, &self.0[1..], value)
    }
}

fn set_tag(current: &mut NbtTag, nodes: &[PathNode], value: NbtTag) -> bool {
    let Some((node, remaining)) = nodes.split_first() else {
        return false;
    };
    match node {
        PathNode::Key(key) => {
            let NbtTag::Compound(compound) = current else {
                return false;
            };
            if remaining.is_empty() {
                compound.child_tags.insert(key.clone().into(), value);
                true
            } else if let Some(child) = compound.child_tags.get_mut(key.as_str()) {
                set_tag(child, remaining, value)
            } else {
                false
            }
        }
        PathNode::Index(index) => {
            let NbtTag::List(list) = current else {
                return false;
            };
            let Some(index) = resolve_index(*index, list.len()) else {
                return false;
            };
            if remaining.is_empty() {
                list[index] = value;
                true
            } else {
                set_tag(&mut list[index], remaining, value)
            }
        }
    }
}

fn resolve_index(index: i32, length: usize) -> Option<usize> {
    let index = if index < 0 {
        i64::try_from(length).ok()?.checked_add(i64::from(index))?
    } else {
        i64::from(index)
    };
    usize::try_from(index).ok().filter(|index| *index < length)
}

pub struct NbtPathArgumentType;

impl ArgumentType for NbtPathArgumentType {
    type Item = NbtPath;

    fn parse(&self, reader: &mut StringReader) -> Result<Self::Item, CommandSyntaxError> {
        let start = reader.cursor();
        let mut nodes = Vec::new();
        let mut expect_key = true;

        while reader.can_read_char() && !reader.peek().is_some_and(char::is_whitespace) {
            if expect_key {
                let key_start = reader.cursor();
                while let Some(c) = reader.peek()
                    && !matches!(c, '.' | '[' | ']')
                    && !c.is_whitespace()
                {
                    reader.skip();
                }
                if reader.cursor() == key_start {
                    return Err(invalid_path(reader, start));
                }
                nodes.push(PathNode::Key(
                    reader.string()[key_start..reader.cursor()].to_string(),
                ));
                expect_key = false;
            }

            while reader.peek() == Some('[') {
                reader.skip();
                let index_start = reader.cursor();
                if reader.peek() == Some('-') {
                    reader.skip();
                }
                while reader.peek().is_some_and(|c| c.is_ascii_digit()) {
                    reader.skip();
                }
                let raw_index = &reader.string()[index_start..reader.cursor()];
                if raw_index.is_empty() || reader.peek() != Some(']') {
                    return Err(invalid_path(reader, start));
                }
                let Ok(index) = raw_index.parse() else {
                    return Err(invalid_path(reader, start));
                };
                reader.skip();
                nodes.push(PathNode::Index(index));
            }

            match reader.peek() {
                Some('.') => {
                    reader.skip();
                    expect_key = true;
                }
                Some(c) if c.is_whitespace() => break,
                None => break,
                _ => return Err(invalid_path(reader, start)),
            }
        }

        if nodes.is_empty() || expect_key {
            Err(invalid_path(reader, start))
        } else {
            Ok(NbtPath(nodes))
        }
    }

    fn client_side_parser(&'_ self) -> JavaClientArgumentType {
        JavaClientArgumentType::NbtPath
    }

    fn examples(&self) -> Vec<String> {
        examples!("foo", "foo.bar", "Motion[0]", "Pos[-1]")
    }
}

fn invalid_path(reader: &StringReader, start: usize) -> CommandSyntaxError {
    INVALID_NODE_ERROR.create(
        reader,
        TextComponent::text(reader.string()[start..reader.cursor()].to_string()),
    )
}

impl NbtPathArgumentType {
    pub fn get<'a>(
        context: &'a CommandContext,
        name: &str,
    ) -> Result<&'a NbtPath, CommandSyntaxError> {
        context.get_argument::<NbtPath>(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_sets_list_value() {
        let path = NbtPathArgumentType
            .parse(&mut StringReader::new("Motion[1]"))
            .unwrap();
        let mut root = NbtCompound::new();
        root.put(
            "Motion",
            NbtTag::List(vec![0.0.into(), 0.0.into(), 0.0.into()]),
        );

        assert!(path.set(&mut root, NbtTag::Double(-0.5)));
        assert_eq!(root.get_list("Motion").unwrap()[1], NbtTag::Double(-0.5));
    }

    #[test]
    fn supports_nested_keys_and_negative_indices() {
        let path = NbtPathArgumentType
            .parse(&mut StringReader::new("outer.values[-1]"))
            .unwrap();
        let mut nested = NbtCompound::new();
        nested.put("values", NbtTag::List(vec![NbtTag::Int(1), NbtTag::Int(2)]));
        let mut root = NbtCompound::new();
        root.put("outer", nested);

        assert!(path.set(&mut root, NbtTag::Int(7)));
        let values = root
            .get_compound("outer")
            .unwrap()
            .get_list("values")
            .unwrap();
        assert_eq!(values[1], NbtTag::Int(7));
    }
}
