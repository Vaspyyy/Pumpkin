use crate::{
    command::{
        CommandError, CommandExecutor, CommandResult, CommandSender,
        args::{
            ConsumedArgs, FindArg, position_3d::Position3DArgumentConsumer,
            simple::SimpleArgConsumer, summonable_entities::SummonableEntitiesArgumentConsumer,
        },
        argument_types::nbt::EXPECTED_COMPOUND_ERROR_TYPE,
        snbt::SnbtParser,
        string_reader::StringReader,
        tree::{CommandTree, builder::argument},
    },
    entity::{NBTStorage, r#type::from_type},
};
use pumpkin_data::translation;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::{math::vector3::Vector3, text::TextComponent};
use uuid::Uuid;

const NAMES: [&str; 1] = ["summon"];

const DESCRIPTION: &str = "Spawns a Entity at position.";

const ARG_ENTITY: &str = "entity";

const ARG_POS: &str = "pos";
const ARG_NBT: &str = "nbt";

struct Executor;

impl CommandExecutor for Executor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let entity_type = SummonableEntitiesArgumentConsumer::find_arg(args, ARG_ENTITY)?;
            let pos = Position3DArgumentConsumer::find_arg(args, ARG_POS);
            let world = sender
                .world_or_first(server)
                .ok_or(CommandError::InvalidRequirement)?;
            let pos = pos.ok().or_else(|| sender.position()).unwrap_or_else(|| {
                let info = world.level_info.load();
                Vector3::new(
                    f64::from(info.spawn_x) + 0.5,
                    f64::from(info.spawn_y) + 1.0,
                    f64::from(info.spawn_z) + 0.5,
                )
            });
            let entity = from_type(entity_type, pos, &world, Uuid::new_v4());
            if let Ok(raw_nbt) = SimpleArgConsumer::find_arg(args, ARG_NBT) {
                let mut reader = StringReader::new(raw_nbt);
                let tag = SnbtParser::parse_for_commands(&mut reader)
                    .map_err(CommandError::SyntaxError)?;
                let NbtTag::Compound(nbt) = tag else {
                    return Err(CommandError::SyntaxError(
                        EXPECTED_COMPOUND_ERROR_TYPE.create_without_context(),
                    ));
                };
                entity.get_entity().read_nbt_non_mut(&nbt).await;
                entity.read_nbt_non_mut(&nbt).await;
                // The explicit summon position takes precedence over a Pos tag.
                entity.get_entity().set_pos(pos);
            }
            let name = entity.get_display_name().await;
            world.spawn_entity(entity).await;
            sender
                .send_message(TextComponent::translate_cross(
                    translation::java::COMMANDS_SUMMON_SUCCESS,
                    translation::bedrock::COMMANDS_SUMMON_SUCCESS,
                    [name],
                ))
                .await;

            Ok(1)
        })
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION).then(
        argument(ARG_ENTITY, SummonableEntitiesArgumentConsumer)
            .execute(Executor)
            .then(
                argument(ARG_POS, Position3DArgumentConsumer)
                    .execute(Executor)
                    .then(argument(ARG_NBT, SimpleArgConsumer).execute(Executor)),
            ),
    )
}
