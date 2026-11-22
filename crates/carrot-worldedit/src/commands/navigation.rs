//! Navigation commands: `/up` — the builders' pillar-up shortcut.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::integer::IntegerArgumentType;
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::EntityBase;
use pumpkin_data::Block;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};
use pumpkin_util::text::TextComponent;

use crate::commands::{ONLY_PLAYERS, player_of, reply};
use crate::session::WeState;

const PERM_NAVIGATION: &str = "carrot:worldedit.navigation";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers `/up` and `//up`.
pub fn register(context: &mut ModuleContext<'_>, _state: &Arc<WeState>) {
    context.register_permission(
        PERM_NAVIGATION,
        "Allows the /up navigation command.",
        OP_TWO,
    );
    for name in ["/up", "//up"] {
        let executor = UpExecutor;
        context.register_command(
            name,
            command(name, "Go upwards some distance.").then(
                argument("distance", IntegerArgumentType::any())
                    .executes(executor.clone())
                    .then(argument("flags", StringArgumentType::GreedyPhrase).executes(executor)),
            ),
            PERM_NAVIGATION,
        );
    }
}

/// `/up <distance> [-f|-g]`: teleports straight up and places glass
/// under the player (`WorldEdit` default; `-f` skips glass, `-g` forces).
#[derive(Clone)]
struct UpExecutor;

impl CommandExecutor for UpExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let Ok(distance) = IntegerArgumentType::get(context, "distance") else {
            reply(context, "distância inválida".to_string());
            return Ok(0);
        };
        let flags = StringArgumentType::get(context, "flags").unwrap_or_default();
        let use_glass = !flags.contains("-f");

        let world = player.world();
        let position = player.position();
        let target_y =
            (position.y.floor() + f64::from(distance)).min(f64::from(world.get_top_y() - 2));
        let glass = Block::from_registry_key("glass")
            .map_or(pumpkin_data::BlockStateId::AIR, |block| {
                block.default_state.id
            });

        let below = pumpkin_util::math::position::BlockPos(Vector3::new(
            position.x.floor() as i32,
            target_y as i32 - 1,
            position.z.floor() as i32,
        ));
        if use_glass && world.is_in_build_limit(below) {
            let current = world.get_block_state_id(&below);
            if current.to_state().is_air() || current.to_state().is_liquid() {
                world.set_block_state(
                    &below,
                    glass,
                    pumpkin_world::world::BlockFlags::NOTIFY_LISTENERS,
                );
            }
        }

        player.teleport(
            Vector3::new(position.x, target_y, position.z),
            None,
            None,
            world,
        );
        player.send_system_message(&TextComponent::text(format!("Para cima {distance}.")));
        Ok(1)
    }
}
