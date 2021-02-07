use super::{utils::*, CommandCounter, ShardManagerContainer, TrackOwner};
use serenity::{
    builder::CreateMessage,
    client::{bridge::gateway::ShardId, Context},
    framework::standard::{macros::command, CommandResult},
    model::{channel::Message, id},
};
use std::time::Instant;

#[command]
#[aliases("s")]
#[description = "Data on the bot"]
// TODO: Display correctly cache size
pub async fn stats(ctx: &Context, msg: &Message) -> CommandResult {
    // Measure time elapsed while seding a message (REST latency)
    let now = Instant::now();
    let mut sand = msg.channel_id.say(&ctx, "Measuring REST latency").await?;
    let http_latency = format!("{}ms", now.elapsed().as_millis());

    let data = ctx.data.read().await;

    // Get WS latency from the ShardManagerContainer
    let ws_latency = {
        let mutex = data.get::<ShardManagerContainer>().unwrap().clone();
        let manager = mutex.lock().await;
        let runners = manager.runners.lock().await;
        let runner = runners.get(&ShardId(ctx.shard_id));
        // Might not have a value, just use ?
        runner
            .map(|r| r.latency.map(|l| format!("{}ms", l.as_millis())))
            .flatten()
            .unwrap_or("?ms".to_owned())
    };

    // TODO: Better way to do this?
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
                    ("Top commands", top_commands, true),
                    ("Cache", cache_stats, false),
                ])
                .footer(|f| {
                    f.text(format!(
                        "Shard: {}/{}, {} guilds",
                        ctx.shard_id + 1,
                        shard_count,
                        guild_count
                    ))
                })
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
        handle_message(
            msg.channel_id
                .send_message(&ctx, |m| {
                    m.embed(|e| e.title("Queue").description(text).colour(colour))
                })
                .await,
        );
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

async fn format_metadata<'a>(
    ctx: &Context,
    gid: id::GuildId,
    meta: songbird::input::Metadata,
    author_id: id::UserId,
    state: Box<songbird::tracks::TrackState>,
) -> CreateMessage<'a> {
    let title = format!("Now playing: {}", meta.title.unwrap_or("".to_owned()));
    let thumb = meta.thumbnail;
    let owner = if let Ok(o) = { author_id.to_user(&ctx).await } {
        o.nick_in(&ctx, gid).await.unwrap_or(o.name)
    } else {
        "?".to_owned()
    };

    let foot = format!("Requested by: {}", owner);
    let mut fields = None;

    {
        let mut out = Vec::new();
        if let Some(a) = meta.artist {
            out.push(("Artist/Channel", a, true));
        }
        if let Some(a) = meta.date {
            let mut d = a;
            d.insert(6, '/');
            d.insert(4, '/');
            out.push(("Date", d, true));
        }
        if out.len() != 0 {
            fields = Some(out)
        }
    }

    let colour = cached_colour(ctx, ctx.cache.guild(gid).await).await;

    let progress_bar = {
        if let Some(d) = meta.duration {
            fn as_mins(s: u64) -> String {
                format!("{}:{}", s / 60, s % 60)
            }
            let p = state.position;
            let d_int = d.as_secs();
            let p_int = p.as_secs();
            let ratio = (p_int as f32 / d_int as f32 * 30.0).round() as u8;
            let mut bar = String::with_capacity(30);
            for _ in 1..ratio {
                bar.push('=')
            }
            bar.push('>');
            for _ in 0..30 - ratio {
                bar.push('-')
            }
            let mut out = String::new();
            out.push_str(&format!(
                "`{}`  `{}`  `{}`",
                as_mins(p_int),
                bar,
                as_mins(d_int)
            ));
            Some(out)
        } else {
            None
        }
    };

    let desc = {
        use songbird::tracks::{LoopState, PlayMode};
        let mut out = String::new();
        out.push_str(&meta.source_url.unwrap_or("".to_owned()));
        if let Some(s) = progress_bar {
            out.push('\n');
            out.push_str(&s);
            out.push('\n');
        } else {
            out.push('\n');
        }
        out.push_str("Status: ");
        out.push_str(match state.playing {
            PlayMode::Play => "Playing",
            PlayMode::Pause => "Paused",
            PlayMode::Stop => "Stopped",
            PlayMode::End => "Ended",
            _ => "?",
        });
        if let LoopState::Finite(l) = state.loops {
            if l != 0 {
                out.push_str(&format!("; {} loops left", l))
            }
        }
        out
    };

    let mut message = CreateMessage::default();
    message.embed(|e| {
        if let Some(f) = fields {
            e.fields(f);
        }
        if let Some(t) = thumb {
            e.thumbnail(t);
        }
        e.title(title)
            .description(desc)
            .footer(|f| f.text(foot))
            .colour(colour)
    });
    message
}
