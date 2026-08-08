use crate::command::{
    CommandSender,
    args::{
        Arg, ArgumentConsumer, ConsumeResult, ConsumedArgs, DefaultNameArgConsumer, FindArg,
        GetClientSideArgParser,
    },
    dispatcher::CommandError,
    tree::RawArgs,
};
use crate::server::Server;
use crate::{command::snbt::SnbtParser, command::string_reader::StringReader};
use pumpkin_data::particle::Particle;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_protocol::java::client::play::{ArgumentType, SuggestionProviders};
use pumpkin_util::identifier::Identifier;

pub struct ParticleArgumentConsumer;

#[derive(Clone)]
pub struct ParsedParticle {
    pub particle: Particle,
    pub data: Vec<u8>,
}

impl ParsedParticle {
    fn parse(name: &str) -> Option<Self> {
        let (name, data) = name
            .find('{')
            .map_or((name, None), |start| (&name[..start], Some(&name[start..])));
        let particle = Particle::from_name(name.strip_prefix("minecraft:").unwrap_or(name))?;

        let data = match particle {
            Particle::Dust => parse_color_data(data, 3, true)?,
            Particle::Flash => parse_color_data(data, 4, false)?,
            _ if data.is_none() => Vec::new(),
            _ => return None,
        };

        Some(Self { particle, data })
    }
}

fn parse_color_data(
    data: Option<&str>,
    color_count: usize,
    include_scale: bool,
) -> Option<Vec<u8>> {
    let defaults = if color_count == 3 {
        vec![1.0, 0.0, 0.0]
    } else {
        vec![1.0; color_count]
    };
    let (color, scale) = if let Some(data) = data {
        let mut reader = StringReader::new(data);
        let NbtTag::Compound(compound) = SnbtParser::parse_for_commands(&mut reader).ok()? else {
            return None;
        };
        let color = compound
            .get_list("color")?
            .iter()
            .map(number_as_f32)
            .collect::<Option<Vec<_>>>()?;
        if color.len() != color_count {
            return None;
        }
        let scale = compound.get("scale").and_then(number_as_f32).unwrap_or(1.0);
        (color, scale)
    } else {
        (defaults, 1.0)
    };

    let mut encoded = Vec::with_capacity((color_count + usize::from(include_scale)) * 4);
    for channel in color {
        encoded.extend_from_slice(&channel.to_be_bytes());
    }
    if include_scale {
        encoded.extend_from_slice(&scale.to_be_bytes());
    }
    Some(encoded)
}

fn number_as_f32(tag: &NbtTag) -> Option<f32> {
    match tag {
        NbtTag::Byte(value) => Some(f32::from(*value)),
        NbtTag::Short(value) => Some(f32::from(*value)),
        NbtTag::Int(value) => Some(*value as f32),
        NbtTag::Long(value) => Some(*value as f32),
        NbtTag::Float(value) => Some(*value),
        NbtTag::Double(value) => Some(*value as f32),
        _ => None,
    }
}

impl GetClientSideArgParser for ParticleArgumentConsumer {
    fn get_client_side_parser(&self) -> ArgumentType {
        ArgumentType::Resource {
            identifier: Identifier::vanilla_static("particle_type"),
        }
    }

    fn get_client_side_suggestion_type_override(&self) -> Option<SuggestionProviders> {
        None
    }
}

impl ArgumentConsumer for ParticleArgumentConsumer {
    fn consume<'a, 'b>(
        &'a self,
        _sender: &'a CommandSender,
        _server: &'a Server,
        args: &'b mut RawArgs<'a>,
    ) -> ConsumeResult<'a> {
        let name_opt: Option<&'a str> = args.pop().map(|arg| arg.value);

        let result: Option<Arg<'a>> = name_opt.and_then(ParsedParticle::parse).map(Arg::Particle);

        Box::pin(async move { result })
    }
}

impl DefaultNameArgConsumer for ParticleArgumentConsumer {
    fn default_name(&self) -> &'static str {
        "particle_type"
    }
}

impl<'a> FindArg<'a> for ParticleArgumentConsumer {
    type Data = &'a ParsedParticle;

    fn find_arg(args: &'a ConsumedArgs, name: &str) -> Result<Self::Data, CommandError> {
        match args.get(name) {
            Some(Arg::Particle(data)) => Ok(data),
            _ => Err(CommandError::InvalidConsumption(Some(name.to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_floats(data: &[u8]) -> Vec<f32> {
        data.chunks_exact(4)
            .map(|bytes| f32::from_be_bytes(bytes.try_into().expect("four-byte float")))
            .collect()
    }

    #[test]
    fn parses_matcha_dust_options() {
        let particle = ParsedParticle::parse("dust{color:[1.000,0.667,0.090],scale:1}")
            .expect("dust options should parse");
        assert_eq!(particle.particle, Particle::Dust);
        assert_eq!(decode_floats(&particle.data), [1.0, 0.667, 0.09, 1.0]);
    }

    #[test]
    fn parses_matcha_flash_options() {
        let particle = ParsedParticle::parse("flash{color:[1.000,1.00,1.000,1.00]}")
            .expect("flash options should parse");
        assert_eq!(particle.particle, Particle::Flash);
        assert_eq!(decode_floats(&particle.data), [1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn rejects_configured_particles_without_supported_data_codec() {
        assert!(ParsedParticle::parse("flame{color:[1,0,0]}").is_none());
    }
}
