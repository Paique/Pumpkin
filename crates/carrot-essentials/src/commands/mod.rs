//! Command implementations of the essentials module.
//!
//! In-game messages are in Brazilian Portuguese, matching the reference
//! server's `EssentialsX` `pt_BR` locale.

mod back;
mod fly;
mod gm;
mod home;
mod nightvision;
mod spawn;
mod speed;
mod tpa;
mod warp;

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_util::text::TextComponent;

use crate::state::EssentialsState;

/// Registers every essentials command and permission node.
pub fn register_all(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    fly::register(context);
    speed::register(context, state);
    gm::register(context);
    tpa::register(context, state);
    home::register(context, state);
    warp::register(context, state);
    back::register(context, state);
    spawn::register(context, state);
    nightvision::register(context);
}

/// Sends a plain-text message to the command source.
pub(crate) fn feedback(context: &CommandContext, message: String) {
    context
        .source
        .send_feedback(TextComponent::text(message), false);
}
