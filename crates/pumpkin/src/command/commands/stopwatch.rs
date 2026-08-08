use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::double::DoubleArgumentType;
use crate::command::argument_types::identifier::IdentifierArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Manages real-time stopwatches.";
const PERMISSION: &str = "minecraft:command.stopwatch";
const ARG_ID: &str = "id";
const ARG_SCALE: &str = "scale";

static ERROR_ALREADY_EXISTS: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_STOPWATCH_ALREADY_EXISTS,
    translation::java::COMMANDS_STOPWATCH_ALREADY_EXISTS,
);
pub(super) static ERROR_DOES_NOT_EXIST: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_STOPWATCH_DOES_NOT_EXIST,
    translation::java::COMMANDS_STOPWATCH_DOES_NOT_EXIST,
);

#[derive(Clone, Copy)]
enum StopwatchOperation {
    Create,
    QueryDefault,
    QueryScaled,
    Restart,
    Remove,
}

struct StopwatchExecutor(StopwatchOperation);

impl CommandExecutor for StopwatchExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let id = context.get_argument::<Identifier>(ARG_ID)?;
            let manager = &context.server().stopwatches;
            match self.0 {
                StopwatchOperation::Create => {
                    if !manager.create(id.clone()).await {
                        return Err(ERROR_ALREADY_EXISTS
                            .create_without_context(TextComponent::text(id.to_string())));
                    }
                    context
                        .source
                        .send_feedback(
                            TextComponent::translate_cross(
                                translation::java::COMMANDS_STOPWATCH_CREATE_SUCCESS,
                                translation::java::COMMANDS_STOPWATCH_CREATE_SUCCESS,
                                [TextComponent::text(id.to_string())],
                            ),
                            true,
                        )
                        .await;
                }
                StopwatchOperation::QueryDefault | StopwatchOperation::QueryScaled => {
                    let Some(elapsed) = manager.elapsed_seconds(id).await else {
                        return Err(ERROR_DOES_NOT_EXIST
                            .create_without_context(TextComponent::text(id.to_string())));
                    };
                    let scale = if matches!(self.0, StopwatchOperation::QueryScaled) {
                        DoubleArgumentType::get(context, ARG_SCALE)?
                    } else {
                        1.0
                    };
                    context
                        .source
                        .send_feedback(
                            TextComponent::translate_cross(
                                translation::java::COMMANDS_STOPWATCH_QUERY,
                                translation::java::COMMANDS_STOPWATCH_QUERY,
                                [
                                    TextComponent::text(id.to_string()),
                                    TextComponent::text(elapsed.to_string()),
                                ],
                            ),
                            true,
                        )
                        .await;
                    return Ok((elapsed * scale) as i32);
                }
                StopwatchOperation::Restart => {
                    if !manager.restart(id).await {
                        return Err(ERROR_DOES_NOT_EXIST
                            .create_without_context(TextComponent::text(id.to_string())));
                    }
                    context
                        .source
                        .send_feedback(
                            TextComponent::translate_cross(
                                translation::java::COMMANDS_STOPWATCH_RESTART_SUCCESS,
                                translation::java::COMMANDS_STOPWATCH_RESTART_SUCCESS,
                                [TextComponent::text(id.to_string())],
                            ),
                            true,
                        )
                        .await;
                }
                StopwatchOperation::Remove => {
                    if !manager.remove(id).await {
                        return Err(ERROR_DOES_NOT_EXIST
                            .create_without_context(TextComponent::text(id.to_string())));
                    }
                    context
                        .source
                        .send_feedback(
                            TextComponent::translate_cross(
                                translation::java::COMMANDS_STOPWATCH_REMOVE_SUCCESS,
                                translation::java::COMMANDS_STOPWATCH_REMOVE_SUCCESS,
                                [TextComponent::text(id.to_string())],
                            ),
                            true,
                        )
                        .await;
                }
            }
            Ok(1)
        })
    }
}

pub(super) struct StopwatchSuggestionProvider;

impl SuggestionProvider for StopwatchSuggestionProvider {
    fn suggest<'a>(
        &'a self,
        context: &'a CommandContext,
        builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult<'a> {
        Box::pin(async move {
            let ids = context
                .server()
                .stopwatches
                .ids()
                .await
                .into_iter()
                .map(|id| id.to_string());
            builder.filter_and_suggest_iter(ids).build()
        })
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &mut PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("stopwatch", DESCRIPTION)
            .requires(PERMISSION)
            .then(
                literal("create").then(
                    argument(ARG_ID, IdentifierArgumentType)
                        .executes(StopwatchExecutor(StopwatchOperation::Create)),
                ),
            )
            .then(
                literal("query").then(
                    argument(ARG_ID, IdentifierArgumentType)
                        .suggests(StopwatchSuggestionProvider)
                        .executes(StopwatchExecutor(StopwatchOperation::QueryDefault))
                        .then(
                            argument(ARG_SCALE, DoubleArgumentType::any())
                                .executes(StopwatchExecutor(StopwatchOperation::QueryScaled)),
                        ),
                ),
            )
            .then(
                literal("restart").then(
                    argument(ARG_ID, IdentifierArgumentType)
                        .suggests(StopwatchSuggestionProvider)
                        .executes(StopwatchExecutor(StopwatchOperation::Restart)),
                ),
            )
            .then(
                literal("remove").then(
                    argument(ARG_ID, IdentifierArgumentType)
                        .suggests(StopwatchSuggestionProvider)
                        .executes(StopwatchExecutor(StopwatchOperation::Remove)),
                ),
            ),
    );
}
