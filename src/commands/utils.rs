use super::TrackOwner;
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

pub async fn permission_check(ctx: &Context, mem: &PartialMember) -> bool {
    for role in &mem.roles {
        if role.to_role_cached(&ctx.cache).await.map_or(false, |r| {
            r.has_permission(Permissions::MANAGE_CHANNELS) || r.name.to_lowercase() == "dj"
        }) {
            return true;
        }
    }
    println!("Permission denied");
    return false;
}

pub fn handle_message<T>(res: SerenityResult<T>) {
    match res {
        Ok(_) => (),
        Err(e) => println!("Could not send/delete message: {}", e),
    }
}

pub async fn format_metadata<'a>(
    ctx: &Context,
    gid: GuildId,
    meta: Metadata,
    author_id: UserId,
    state: Box<TrackState>,
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
        let mut inline = true;
        if let Some(a) = meta.artist {
            out.push(("Artist/Channel", a, inline));
            inline = !inline;
        }
        if let Some(a) = meta.date {
            let mut d = a;
            d.insert(6, '/');
            d.insert(4, '/');
            out.push(("Date", d, inline));
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

pub async fn cached_colour(ctx: &Context, guild: Option<Guild>) -> Colour {
    if let Some(g) = guild {
        if let Ok(me) = g.member(&ctx, ctx.cache.current_user_id().await).await {
            return me.colour(&ctx.cache).await.unwrap_or(Colour(0xffffff));
        }
    };
    Colour(0xffffff)
}

pub async fn enqueue(
    ctx: &Context,
    msg: &Message,
    input: songbird::input::Input,
) -> Result<(), ()> {
    let guild = msg.guild(&ctx.cache).await.unwrap();
    let guild_id = guild.id;
    let channel_id = match guild
        .voice_states
        .get(&msg.author.id)
        .and_then(|vs| vs.channel_id)
    {
        Some(id) => id,
        None => {
            handle_message(msg.reply(&ctx, "not in a voice channel").await);
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

    let locked = manager.get(guild_id).unwrap();
    let mut call = locked.lock().await;

    let (track, track_handle) = tracks::create_player(input);
    let mut typemap = track_handle.typemap().write().await;
    typemap.insert::<TrackOwner>(msg.author.id);
    call.enqueue(track);

    Ok(())
}
