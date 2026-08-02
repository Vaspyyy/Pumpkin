use crate::command::argument_builder::{ArgumentBuilder, argument, command};
use crate::command::argument_types::identifier::IdentifierArgumentType;
use crate::command::context::command_context::CommandContext;
use crate::command::errors::error_types::{CommandErrorType, DISPATCHER_PARSE_EXCEPTION};
use crate::command::node::dispatcher::CommandDispatcher;
use crate::command::node::{CommandExecutor, CommandExecutorResult};
use crate::command::suggestion::provider::{SuggestionProvider, SuggestionProviderResult};
use crate::command::suggestion::suggestions::SuggestionsBuilder;
use pumpkin_data::translation;
use pumpkin_util::PermissionLvl;
use pumpkin_util::identifier::Identifier;
use pumpkin_util::permission::{Permission, PermissionDefault, PermissionRegistry};
use pumpkin_util::text::TextComponent;

const DESCRIPTION: &str = "Runs a data pack function.";
const PERMISSION: &str = "minecraft:command.function";
const ARG_NAME: &str = "name";

static UNKNOWN_FUNCTION: CommandErrorType<1> = CommandErrorType::new(
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
    translation::java::ARGUMENTS_FUNCTION_UNKNOWN,
);

struct FunctionExecutor;

impl CommandExecutor for FunctionExecutor {
    fn execute<'a>(&'a self, context: &'a CommandContext) -> CommandExecutorResult<'a> {
        Box::pin(async move {
            let id = context.get_argument::<Identifier>(ARG_NAME)?;
            let server = context.server();
            if server.data_pack_manager.function(id).is_none() {
                return Err(
                    UNKNOWN_FUNCTION.create_without_context(TextComponent::text(id.to_string()))
                );
            }

            let result = server
                .data_pack_manager
                .execute_function(server, id, &context.source)
                .await
                .map_err(|error| {
                    DISPATCHER_PARSE_EXCEPTION
                        .create_without_context(TextComponent::text(error.to_string()))
                })?;

            context
                .source
                .send_feedback(
                    TextComponent::translate_cross(
                        translation::java::COMMANDS_FUNCTION_SUCCESS_SINGLE,
                        translation::java::COMMANDS_FUNCTION_SUCCESS_SINGLE,
                        [
                            TextComponent::text(result.to_string()),
                            TextComponent::text(id.to_string()),
                        ],
                    ),
                    true,
                )
                .await;
            Ok(result)
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
        let ids = context
            .server()
            .data_pack_manager
            .function_ids()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        Box::pin(async move { builder.filter_and_suggest_iter(ids).build() })
    }
}

pub fn register(dispatcher: &mut CommandDispatcher, registry: &mut PermissionRegistry) {
    registry.register_permission_or_panic(Permission::new(
        PERMISSION,
        DESCRIPTION,
        PermissionDefault::Op(PermissionLvl::Two),
    ));

    dispatcher.register(
        command("function", DESCRIPTION).requires(PERMISSION).then(
            argument(ARG_NAME, IdentifierArgumentType)
                .suggests(FunctionSuggestionProvider)
                .executes(FunctionExecutor),
        ),
    );
}
