use super::{CommandCounter, ShardManagerContainer};
use serenity::{
    client::Context,
    framework::standard::{macros::command, CommandResult},
    model::{channel::Message, id::UserId},
    prelude::TypeMapKey,
};
use utils::*;

pub mod add;
pub mod display;
pub mod hooks;
pub mod queue;
mod utils;

pub use add::*;
pub use display::*;
pub use hooks::*;
pub use queue::*;

struct TrackOwner;

impl TypeMapKey for TrackOwner {
    type Value = UserId;
}
#[command]
#[aliases("l")]
#[only_in(guilds)]
#[description = "Leave the voice channel, flushing the queue"]
// TODO: Check for user in channel
pub async fn leave(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(m) = &msg.member {
        if !permission_check(ctx, m).await {
            return Ok(());
        }
    } else {
        println!("Permission check failed for user {}", msg.author.id.0);
        return Ok(());
    }

    let guild = msg.guild(&ctx.cache).await.unwrap();
    let guild_id = guild.id;

    let manager = songbird::get(ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(guild_id) {
        let _ = lock.lock().await.queue().stop();
        if let Err(e) = manager.remove(guild_id).await {
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Failed: {:?}", e))
                    .await,
            );
        }
    }

    Ok(())
}

#[command]
#[aliases("j")]
#[only_in(guilds)]
#[description = "Join the voice channel"]
// TODO: Check if already playing
async fn join(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg.guild(&ctx.cache).await.unwrap();
    let guild_id = guild.id;

    let channel_id = match guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|vs| vs.channel_id)
    {
        Some(id) => id,
        None => {
            msg.reply(&ctx, "not in a voice channel").await?;
            return Ok(());
        }
    };

    let manager = songbird::get(ctx).await.unwrap().clone();

    if manager.get(guild_id).is_none() {
        let (_, join_result) = manager.join(guild_id, channel_id).await;
        if let Err(_) = join_result {
            msg.channel_id
                .say(&ctx, "Couldn't join voice channel")
                .await
                .unwrap();
        }
    }

    let _handler = manager.join(guild_id, channel_id).await;
    Ok(())
}
