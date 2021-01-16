use super::{CommandCounter, ShardManagerContainer};
use utils::*;
use futures::{prelude::stream, stream::StreamExt};
use serde_json::Value;
use serenity::{
    builder::CreateMessage,
    client::{bridge::gateway::ShardId, Context},
    framework::standard::{macros::command, Args, CommandResult},
    model::{
        channel::Message,
        guild::{Guild, PartialMember},
        id::{GuildId, UserId},
        permissions::Permissions,
    },
    prelude::TypeMapKey,
    utils::Colour,
    Result as SerenityResult,
};
use songbird::{
    input::Metadata,
    tracks::{self, TrackState},
};
use std::time::Instant;

pub mod hooks;
mod utils;

pub use hooks::*;

struct TrackOwner;

impl TypeMapKey for TrackOwner {
    type Value = UserId;
}

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
#[aliases("a")]
#[only_in(guilds)]
#[min_args(1)]
#[description = "Add song to queue"]
// TODO: Handle playlists
pub async fn add(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let query = args
        .iter()
        .map(|a| a.unwrap_or("".to_owned()))
        .collect::<Vec<String>>()
        .join(" ");

    let (input, query_msg) = match if query.starts_with("http") {
        (
            msg.channel_id
                .say(&ctx.http, format!("Adding {} to the queue", query))
                .await,
            songbird::ytdl(&query).await,
        )
    } else {
        (
            msg.channel_id
                .say(&ctx.http, format!("Searching on Youtube {}", query))
                .await,
            songbird::input::ytdl_search(&query).await,
        )
    } {
        (m, Ok(i)) => (i, m.unwrap()),
        (_, Err(e)) => {
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Error: {:?}", e))
                    .await,
            );
            return Ok(());
        }
    };

    enqueue(ctx, msg, input).await.unwrap();
    handle_message(query_msg.delete(&ctx.http).await);

    Ok(())
}

#[command]
#[aliases("r", "addraw", "add-raw", "ar")]
#[only_in(guilds)]
#[min_args(1)]
#[description = "Add ffmpeg URI to the queue"]
pub async fn raw(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let query: String = args.single().unwrap();

    let (input, query_msg) = match if {
        query.starts_with("http")
            || query.starts_with("rtmp")
            || query.starts_with("ftp")
            || query.starts_with("hls")
            || query.starts_with("tcp")
            || query.starts_with("udp")
    } {
        (
            msg.channel_id
                .say(&ctx.http, format!("Adding {} to the queue", query))
                .await,
            songbird::ffmpeg(&query).await,
        )
    } else {
        handle_message(
            msg.channel_id
                .say(&ctx.http, format!("Invalid protocol"))
                .await,
        );
        return Ok(());
    } {
        (m, Ok(i)) => (i, m.unwrap()),
        (_, Err(e)) => {
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Error: {:?}", e))
                    .await,
            );
            return Ok(());
        }
    };

    enqueue(ctx, msg, input).await.unwrap();
    handle_message(query_msg.delete(&ctx.http).await);

    Ok(())
}

#[command]
#[aliases("i", "ice", "ai", "add-icecast")]
#[only_in(guilds)]
#[min_args(1)]
#[description = "Add icecast stream to the queue"]
// TODO: Parse start time as SystemTime
pub async fn icecast(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    use crate::icecast::FromIceJson;

    let query: String = args.single().unwrap();

    let (input, query_msg) = match if query.starts_with("http") {
        (
            msg.channel_id
                .say(&ctx.http, format!("Adding {} to the queue", query))
                .await,
            {
                let uri: http::uri::Uri = query.parse().unwrap();
                let stats = format!(
                    "{}://{}/status-json.xsl",
                    uri.scheme_str().unwrap(),
                    uri.authority().unwrap(),
                );
                let json: Value = reqwest::get(&stats).await?.json().await?;
                songbird::ffmpeg(&query).await.and_then(|mut i| {
                    i.metadata = Box::new(Metadata::from_ice_json(json, &query));
                    Ok(i)
                })
            },
        )
    } else {
        handle_message(
            msg.channel_id
                .say(&ctx.http, format!("Invalid protocol"))
                .await,
        );
        return Ok(());
    } {
        (m, Ok(i)) => (i, m.unwrap()),
        (_, Err(e)) => {
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Error: {:?}", e))
                    .await,
            );
            return Ok(());
        }
    };

    enqueue(ctx, msg, input).await.unwrap();
    handle_message(query_msg.delete(&ctx.http).await);

    Ok(())
}

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
            lock.lock()
                .await
                .queue()
                .modify_queue(|queue| queue.remove(i));
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
