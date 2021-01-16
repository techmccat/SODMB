use super::{utils::*, CommandCounter, ShardManagerContainer, TrackOwner};
use serenity::{
    client::{bridge::gateway::ShardId, Context},
    framework::standard::{macros::command, CommandResult},
    model::channel::Message,
};
use std::time::Instant;

#[command]
#[aliases("s")]
#[description = "Data on the bot"]
// TODO: Display correctly cache size
pub async fn stats(ctx: &Context, msg: &Message) -> CommandResult {
    let now = Instant::now();
    let mut sand = msg.channel_id.say(&ctx, "Measuring REST latency").await?;
    let http_latency = format!("{}ms", now.elapsed().as_millis());

    let data = ctx.data.read().await;
    let ws_latency = {
        let mutex = data.get::<ShardManagerContainer>().unwrap().clone();
        let manager = mutex.lock().await;
        let runners = manager.runners.lock().await;
        let runner = runners.get(&ShardId(ctx.shard_id));
        if let Some(r) = runner {
            if let Some(l) = r.latency {
                format!("{}ms", l.as_millis())
            } else {
                "?ms".to_owned()
            }
        } else {
            "?ms".to_owned()
        }
    };

    let top_commands = {
        let map = data.get::<CommandCounter>().unwrap().clone();
        let mut count: Vec<(&String, &u64)> = map.iter().collect();
        count.sort_by(|a, b| b.1.cmp(a.1));
        let lines: Vec<String> = count
            .iter()
            .enumerate()
            .filter(|(a, _)| *a < 3)
            .map(|(_, (cmd, count))| format!("{} - {}", count, cmd))
            .collect();
        lines.join("\n")
    };

    let cache_stats = {
        let mut out = String::new();
        out.push_str(&format!(
            "Cached guilds: {}\n",
            ctx.cache.guilds().await.len()
        ));
        out.push_str(&format!(
            "Cached channels: {}\n",
            ctx.cache.guild_channel_count().await
        ));
        out.push_str(&format!("Cached users: {}\n", ctx.cache.user_count().await));
        // out.push_str(&format!("Cache size: {}B\n", size_of_val(&ctx.cache)));
        out
    };

    let author = msg
        .author_nick(&ctx)
        .await
        .unwrap_or(msg.author.name.clone());
    let shard_count = ctx.cache.shard_count().await;
    let guild_count = format!("{}", ctx.cache.guilds().await.len());
    let colour = cached_colour(ctx, msg.guild(&ctx.cache).await).await;

    sand.edit(&ctx, |m| {
        m.content("").embed(|e| {
            e.title("Stats")
                .description(format!("Called by {}", author))
                .fields(vec![
                    (
                        "Latency",
                        format!("Gateway: {}\nREST API: {}", ws_latency, http_latency),
                        true,
                    ),
                    ("Guilds", guild_count, true),
                    ("Top commands", top_commands, true),
                    ("Cache", cache_stats, false),
                ])
                .footer(|f| f.text(format!("Shard: {}/{}", ctx.shard_id + 1, shard_count)))
                .colour(colour)
        })
    })
    .await?;
    Ok(())
}

#[command]
#[aliases("q")]
#[only_in(guilds)]
#[description = "Print song queue"]
pub async fn queue(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg.guild_id.unwrap();
    let manager = songbird::get(ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(guild_id) {
        let call = lock.lock().await;
        let queue = call.queue().current_queue();
        let text = {
            let mut out = Vec::with_capacity(queue.len());
            for (i, e) in queue.iter().enumerate().take(16) {
                let meta = e.metadata().clone();
                let owner = if let Ok(o) = {
                    let read = e.typemap().read().await;
                    let user_id = read.get::<TrackOwner>().unwrap();
                    user_id.to_user(&ctx).await
                } {
                    o.nick_in(&ctx, guild_id).await.unwrap_or(o.name)
                } else {
                    "?".to_owned()
                };
                out.push(format!(
                    "`{}`: [{}]({}) {}\nRequested by {}",
                    i,
                    meta.title.unwrap_or("?".to_owned()),
                    meta.source_url.unwrap_or("?".to_owned()),
                    match meta.duration {
                        Some(d) => {
                            let s = d.as_secs();
                            format!("{}:{}", s / 60, s % 60)
                        }
                        None => "?".to_owned(),
                    },
                    owner
                ))
            }
            out.join("\n")
        };
        let colour = cached_colour(ctx, msg.guild(&ctx.cache).await).await;
        msg.channel_id
            .send_message(&ctx, |m| {
                m.embed(|e| e.title("Queue").description(text).colour(colour))
            })
            .await
            .unwrap();
    } else {
        handle_message(msg.channel_id.say(&ctx, "Not in a voice channel").await);
    }

    Ok(())
}

#[command]
#[aliases("n")]
#[only_in(guilds)]
#[description = "Show details on the current song"]
// TODO: Check if already playing
async fn np(ctx: &Context, msg: &Message) -> CommandResult {
    let guild_id = msg.guild_id.unwrap();
    let manager = songbird::get(ctx).await.unwrap().clone();

    if let Some(lock) = manager.get(guild_id) {
        let call = lock.lock().await;
        let current = if let Some(t) = call.queue().current() {
            t
        } else {
            handle_message(msg.channel_id.say(&ctx.http, "No song playing").await);
            return Ok(());
        };
        let meta = current.metadata().clone();
        let owner = {
            let read = current.typemap().read().await;
            *read.get::<TrackOwner>().unwrap()
        };
        let state = current.get_info().await.unwrap();
        let mut message = format_metadata(&ctx, guild_id, meta, owner, state).await;
        handle_message(msg.channel_id.send_message(&ctx, |_| &mut message).await);
    } else {
        handle_message(msg.channel_id.say(&ctx, "Not in a voice channel").await);
    }

    Ok(())
}
