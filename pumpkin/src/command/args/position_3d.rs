use pumpkin_protocol::java::client::play::{ArgumentType, SuggestionProviders};
use pumpkin_util::math::vector3::Vector3;

use crate::command::CommandSender;
use crate::command::args::ConsumeResult;
use crate::command::dispatcher::CommandError;
use crate::command::tree::RawArgs;
use crate::server::Server;

use super::super::args::ArgumentConsumer;
use super::coordinate::MaybeRelativeCoordinate;
use super::position_block::{apply_local_coordinates, parse_local_coordinate};
use super::{Arg, DefaultNameArgConsumer, FindArg, GetClientSideArgParser};

/// x, y and z coordinates
pub struct Position3DArgumentConsumer;

impl GetClientSideArgParser for Position3DArgumentConsumer {
    fn get_client_side_parser(&self) -> ArgumentType {
        ArgumentType::Vec3
    }

    fn get_client_side_suggestion_type_override(&self) -> Option<SuggestionProviders> {
        None
    }
}

impl ArgumentConsumer for Position3DArgumentConsumer {
    fn consume<'a, 'b>(
        &'a self,
        sender: &'a CommandSender,
        _server: &'a Server,
        args: &'b mut RawArgs<'a>,
    ) -> ConsumeResult<'a> {
        let x_str_opt = args.pop().map(|arg| arg.value);
        let y_str_opt = args.pop().map(|arg| arg.value);
        let z_str_opt = args.pop().map(|arg| arg.value);

        let (Some(x_str), Some(y_str), Some(z_str)) = (x_str_opt, y_str_opt, z_str_opt) else {
            return Box::pin(async move { None });
        };

        let result: Option<Arg<'a>> = MaybeRelativePosition3D::try_new(x_str, y_str, z_str)
            .and_then(|pos| pos.try_to_absolute(sender))
            .map(Arg::Pos3D);

        Box::pin(async move { result })
    }
}

enum MaybeRelativePosition3D {
    World(
        MaybeRelativeCoordinate<false>,
        MaybeRelativeCoordinate<true>,
        MaybeRelativeCoordinate<false>,
    ),
    Local(f64, f64, f64),
}

impl MaybeRelativePosition3D {
    fn try_new(x: &str, y: &str, z: &str) -> Option<Self> {
        if x.starts_with('^') || y.starts_with('^') || z.starts_with('^') {
            Some(Self::Local(
                parse_local_coordinate(x)?,
                parse_local_coordinate(y)?,
                parse_local_coordinate(z)?,
            ))
        } else {
            Some(Self::World(
                x.try_into().ok()?,
                y.try_into().ok()?,
                z.try_into().ok()?,
            ))
        }
    }

    fn try_to_absolute(self, sender: &CommandSender) -> Option<Vector3<f64>> {
        match self {
            Self::World(x, y, z) => {
                let origin = sender.position();
                Some(Vector3::new(
                    x.into_absolute(origin.map(|o| o.x))?,
                    y.into_absolute(origin.map(|o| o.y))?,
                    z.into_absolute(origin.map(|o| o.z))?,
                ))
            }
            Self::Local(left, up, forwards) => {
                let origin = sender.position()?;
                let rotation = sender.rotation()?;
                let offset = apply_local_coordinates(rotation, Vector3::new(left, up, forwards));
                Some(origin.add(&offset))
            }
        }
    }
}

impl DefaultNameArgConsumer for Position3DArgumentConsumer {
    fn default_name(&self) -> &'static str {
        "pos"
    }
}

impl<'a> FindArg<'a> for Position3DArgumentConsumer {
    type Data = Vector3<f64>;

    fn find_arg(args: &'a super::ConsumedArgs, name: &str) -> Result<Self::Data, CommandError> {
        match args.get(name) {
            Some(Arg::Pos3D(data)) => Ok(*data),
            _ => Err(CommandError::InvalidConsumption(Some(name.to_string()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use pumpkin_util::math::vector2::Vector2;

    use super::*;
    use crate::command::context::command_source::CommandSource;
    use crate::command::with_legacy_command_source;

    #[tokio::test]
    async fn local_coordinates_use_bridged_source() {
        let mut source = CommandSource::dummy();
        source.position = Vector3::new(10.0, 20.0, 30.0);
        // Modern command sources store pitch first and yaw second.
        source.rotation = Vector2::new(0.0, 90.0);

        let resolved = with_legacy_command_source(&source, async {
            MaybeRelativePosition3D::try_new("^", "^", "^0.1")
                .and_then(|position| position.try_to_absolute(&CommandSender::Dummy))
        })
        .await
        .expect("local position should resolve");

        assert!((resolved.x - 9.9).abs() < f64::EPSILON);
        assert!((resolved.y - 20.0).abs() < f64::EPSILON);
        assert!((resolved.z - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn local_and_world_coordinates_cannot_be_mixed() {
        assert!(MaybeRelativePosition3D::try_new("^", "~", "^").is_none());
    }
}
