use crate::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use crate::command::argument_types::core::string::StringArgumentType;
use crate::command::argument_types::function::FunctionArgumentType;
use crate::command::argument_types::time::TimeArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::CommandErrorType;
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use crate::server::function_scheduler::ScheduledFunctionKind;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Schedules a data pack function.";
const PERMISSION: &str = "minecraft:command.schedule";
const ARG_FUNCTION: &str = "function";
const ARG_TIME: &str = "time";

static UNKNOWN_FUNCTION: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
);
static CLEAR_FAILED: CommandErrorType<1> = CommandErrorType::new(
    translation::java::COMMANDS_SCHEDULE_CLEARED_FAILURE,
    translation::java::COMMANDS_SCHEDULE_CLEARED_FAILURE,
);

struct ScheduleExecutor {
    replace: bool,
}

impl CommandExecutor for ScheduleExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let reference = FunctionArgumentType::get(context, ARG_FUNCTION)?.clone();
            let delay = TimeArgumentType::get(context, ARG_TIME)?;
            let server = context.server();
            let exists = if reference.is_tag {
                server.data_pack_manager.has_tag(&reference.id)
            } else {
                server.data_pack_manager.function(&reference.id).is_some()
            };
            if !exists {
                let display = if reference.is_tag {
                    format!("#{}", reference.id)
                } else {
                    reference.id.to_string()
                };
                return Err(UNKNOWN_FUNCTION.create_without_context(TextComponent::text(display)));
            }

            let kind = if reference.is_tag {
                ScheduledFunctionKind::Tag
            } else {
                ScheduledFunctionKind::Function
            };
            let trigger_time = server
                .function_scheduler
                .schedule(reference.id.clone(), kind, delay, self.replace)
                .await;
            let translation_key = if reference.is_tag {
                translation::java::COMMANDS_SCHEDULE_CREATED_TAG
            } else {
                translation::java::COMMANDS_SCHEDULE_CREATED_FUNCTION
            };
            context
                .source
                .send_feedback(
                    TextComponent::translate_cross(
                        translation_key,
                        translation_key,
                        [
                            TextComponent::text(reference.id.to_string()),
                            TextComponent::text(delay.to_string()),
                            TextComponent::text(trigger_time.to_string()),
                        ],
                    ),
                    true,
                )
                .await;
            Ok(trigger_time.rem_euclid(i64::from(i32::MAX)) as i32)
        })
    }
}

struct ClearExecutor;

impl CommandExecutor for ClearExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let name = StringArgumentType::get(context, ARG_FUNCTION)?;
            let removed = context.server().function_scheduler.clear(name).await;
            if removed == 0 {
                return Err(
                    CLEAR_FAILED.create_without_context(TextComponent::text(name.to_string()))
                );
            }
            context
                .source
                .send_feedback(
                    TextComponent::translate_cross(
                        translation::java::COMMANDS_SCHEDULE_CLEARED_SUCCESS,
                        translation::java::COMMANDS_SCHEDULE_CLEARED_SUCCESS,
                        [
                            TextComponent::text(removed.to_string()),
                            TextComponent::text(name.to_string()),
                        ],
                    ),
                    true,
                )
                .await;
            Ok(i32::try_from(removed).unwrap_or(i32::MAX))
        })
    }
}

struct FunctionSuggestionProvider;

impl SuggestionProvider for FunctionSuggestionProvider {
    fn suggest<'a>(
        &'a self,
        context: &'a CommandContext,
        builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult<'a> {
        let manager = &context.server().data_pack_manager;
        let suggestions = manager
            .function_ids()
            .map(ToString::to_string)
            .chain(manager.tag_ids().map(|id| format!("#{id}")))
            .collect::<Vec<_>>();
        Box::pin(async move { builder.filter_and_suggest_iter(suggestions).build() })
    }
}

struct ScheduledEventSuggestionProvider;

impl SuggestionProvider for ScheduledEventSuggestionProvider {
    fn suggest<'a>(
        &'a self,
        context: &'a CommandContext,
        builder: SuggestionsBuilder,
    ) -> SuggestionProviderResult<'a> {
        Box::pin(async move {
            let names = context.server().function_scheduler.event_names().await;
            builder.filter_and_suggest_iter(names).build()
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
        command("schedule", DESCRIPTION)
            .requires(PERMISSION)
            .then(
                literal("function").then(
                    argument(ARG_FUNCTION, FunctionArgumentType)
                        .suggests(FunctionSuggestionProvider)
                        .then(
                            argument(ARG_TIME, TimeArgumentType::new(1))
                                .executes(ScheduleExecutor { replace: true })
                                .then(
                                    literal("append").executes(ScheduleExecutor { replace: false }),
                                )
                                .then(
                                    literal("replace").executes(ScheduleExecutor { replace: true }),
                                ),
                        ),
                ),
            )
            .then(
                literal("clear").then(
                    argument(ARG_FUNCTION, StringArgumentType::GreedyPhrase)
                        .suggests(ScheduledEventSuggestionProvider)
                        .executes(ClearExecutor),
                ),
            ),
    );
}
