use crate::command::{
    CommandError, CommandExecutor, CommandResult, CommandSender,
    args::{
        ConsumedArgs, FindArg, bounded_num::BoundedNumArgumentConsumer,
        position_3d::Position3DArgumentConsumer, resource::particle::ParticleArgumentConsumer,
    },
    tree::{
        CommandTree,
        builder::{argument, literal},
    },
};
use pumpkin_util::{math::vector3::Vector3, text::TextComponent};
const NAMES: [&str; 1] = ["particle"];

const DESCRIPTION: &str = "Spawns a Particle at position.";

const ARG_NAME: &str = "name";

const ARG_POS: &str = "pos";
const ARG_DELTA: &str = "delta";
const ARG_SPEED: &str = "speed";
const ARG_COUNT: &str = "count";

#[derive(Clone, Copy)]
struct Executor {
    force: bool,
}

impl Executor {
    const NORMAL: Self = Self { force: false };
    const FORCE: Self = Self { force: true };
}

impl CommandExecutor for Executor {
    fn execute<'a>(
        &'a self,
        sender: &'a CommandSender,
        server: &'a crate::server::Server,
        args: &'a ConsumedArgs<'a>,
    ) -> CommandResult<'a> {
        Box::pin(async move {
            let particle = ParticleArgumentConsumer::find_arg(args, ARG_NAME)?;
            let pos = Position3DArgumentConsumer::find_arg(args, ARG_POS);
            let delta = Position3DArgumentConsumer::find_arg(args, ARG_DELTA);
            let speed = BoundedNumArgumentConsumer::<f32>::find_arg(args, ARG_SPEED);
            let count = BoundedNumArgumentConsumer::<i32>::find_arg(args, ARG_COUNT);

            let delta = delta.unwrap_or(Vector3::new(0.0, 0.0, 0.0));
            let delta: Vector3<f32> = Vector3::new(delta.x as f32, delta.y as f32, delta.z as f32);
            let speed = speed.unwrap_or(Ok(0.0))?;
            let count = count.unwrap_or(Ok(0))?;
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

            world.spawn_particle_with_data(
                pos,
                delta,
                speed,
                count,
                particle.particle,
                self.force,
                self.force,
                &particle.data,
            );

            sender
                .send_message(TextComponent::translate_cross(
                    pumpkin_data::translation::java::COMMANDS_PARTICLE_SUCCESS,
                    pumpkin_data::translation::bedrock::COMMANDS_PARTICLE_SUCCESS,
                    [
                        TextComponent::text(format!("{:?}", particle.particle)),
                        TextComponent::text(count.to_string()),
                    ],
                ))
                .await;

            // TODO: Add `viewers` arguments and change the logic for this result
            Ok(1)
        })
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION).then(
        argument(ARG_NAME, ParticleArgumentConsumer)
            .execute(Executor::NORMAL)
            .then(
                argument(ARG_POS, Position3DArgumentConsumer)
                    .execute(Executor::NORMAL)
                    .then(
                        argument(ARG_DELTA, Position3DArgumentConsumer)
                            .execute(Executor::NORMAL)
                            .then(
                                argument(
                                    ARG_SPEED,
                                    BoundedNumArgumentConsumer::<f32>::new().min(0.0),
                                )
                                .execute(Executor::NORMAL)
                                .then(
                                    argument(
                                        ARG_COUNT,
                                        BoundedNumArgumentConsumer::<i32>::new().min(0),
                                    )
                                    .execute(Executor::NORMAL)
                                    .then(literal("normal").execute(Executor::NORMAL))
                                    .then(literal("force").execute(Executor::FORCE)),
                                ),
                            ),
                    ),
            ),
        // TODO: Add NBT
    )
}
