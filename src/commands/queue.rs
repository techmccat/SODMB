use super::{utils::*, TrackOwner};
use futures::{prelude::stream, stream::StreamExt};
use serenity::{
    client::Context,
    framework::standard::{macros::command, Args, CommandResult},
    model::channel::Message,
};

#[command]
#[aliases("p")]
#[only_in(guilds)]
#[description = "Pause playback"]
// TODO: Check for user in channel
pub async fn pause(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(m) = &msg.member {
        if !permission_check(ctx, m).await {
            return Ok(());
        }
    } else {
        return Ok(());
    }

    let manager = songbird::get(&ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(msg.guild_id.unwrap()) {
        let call = lock.lock().await;
        let _ = call.queue().pause();
    }

    Ok(())
}

#[command]
#[aliases("continue", "cont", "c")]
#[only_in(guilds)]
#[description = "Resume playback"]
// TODO: Check for user in channel
pub async fn play(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(m) = &msg.member {
        if !permission_check(ctx, m).await {
            return Ok(());
        }
    } else {
        return Ok(());
    }

    let manager = songbird::get(&ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(msg.guild_id.unwrap()) {
        let call = lock.lock().await;
        let _ = call.queue().resume();
    }

    Ok(())
}

#[command]
#[aliases("s")]
#[only_in(guilds)]
#[description = "Skip one song"]
// TODO: Check for user in channel
// TODO: Implement poll for non-privileged users
pub async fn skip(ctx: &Context, msg: &Message) -> CommandResult {
    if let Some(m) = &msg.member {
        if !permission_check(ctx, m).await {
            return Ok(());
        }
    } else {
        return Ok(());
    }

    let manager = songbird::get(&ctx).await.unwrap().clone();
    if let Some(lock) = manager.get(msg.guild_id.unwrap()) {
        let call = lock.lock().await;
        let _ = call.queue().skip();
    }

    Ok(())
}

#[command]
#[aliases("c")]
#[only_in(guilds)]
#[description = "Clear song queue"]
// TODO: Check for user in channel
pub async fn clear(ctx: &Context, msg: &Message, _: Args) -> CommandResult {
    if let Some(m) = &msg.member {
        if !permission_check(ctx, m).await {
            return Ok(());
        }
    } else {
        return Ok(());
    }

    let manager = songbird::get(ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(msg.guild_id.unwrap()) {
        let call = lock.lock().await;
        let _ = call.queue().stop();
    }

    Ok(())
}

#[command]
#[aliases("p")]
#[only_in(guilds)]
#[description = "Remove your last requested song"]
pub async fn pop(ctx: &Context, msg: &Message) -> CommandResult {
    let guild = msg.guild(&ctx.cache).await.unwrap();
    let guild_id = guild.id;

    let manager = songbird::get(ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(guild_id) {
        if let Some((i, removed)) = {
            let call = lock.lock().await;
            let vec = call.queue().current_queue();
            // iterate on current queue, select only songs by user, take the last one
            stream::iter(vec.iter().enumerate())
                .filter_map(|(i, e)| async move {
                    let read = e.typemap().read().await;
                    let owner = read.get::<TrackOwner>().unwrap();
                    if *owner == msg.author.id {
                        Some((i, e.clone()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<(usize, songbird::tracks::TrackHandle)>>()
                .await
                .pop()
        } {
            // Actually remove the track from the queue, stop it to prevent leaks
            lock.lock()
                .await
                .queue()
                .modify_queue(|queue| queue.remove(i).and_then(|track| track.stop().ok()));

            let nick = msg
                .author_nick(&ctx)
                .await
                .unwrap_or(msg.author.name.clone());
            let meta = removed.metadata().clone();
            let url = meta.source_url.unwrap();
            let title = meta.title.unwrap_or(url.clone());
            let desc = format!("`{}`: [{}]({})\nRequested by {}", i, title, url, nick);
            let colour = cached_colour(ctx, msg.guild(&ctx.cache).await).await;

            handle_message(
                msg.channel_id
                    .send_message(&ctx, |m| {
                        m.embed(|e| {
                            e.title(format!("Removed queue entry {}", i))
                                .description(desc)
                                .colour(colour)
                        })
                    })
                    .await,
            )
        }
    }
    Ok(())
}
