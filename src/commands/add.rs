use super::{utils::*, TrackOwner};
use serde_json::Value;
use serenity::{
    client::Context,
    framework::standard::{macros::command, Args, CommandResult},
    model::channel::Message,
};
use songbird::{
    input::{cached::Compressed, Metadata},
    Bitrate,
};
use std::time::Duration;
use tracing::{info, warn};

#[cfg(feature = "cache")]
use crate::cache::{self, TrackCache, TrackEndEvent, BITRATE};
#[cfg(feature = "cache")]
use songbird::{Event, TrackEvent};
#[cfg(feature = "cache")]
use tokio::{fs::File, io::AsyncReadExt};

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
            info!("Error creating input: {:?}", e);
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Error: {:?}", e))
                    .await,
            );
            return Ok(());
        }
    };

    enqueue(ctx, msg, input).await;
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
            info!("Error creating input: {:?}", e);
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Error: {:?}", e))
                    .await,
            );
            return Ok(());
        }
    };

    enqueue(ctx, msg, input).await;
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
            info!("Error creating input: {:?}", e);
            handle_message(
                msg.channel_id
                    .say(&ctx.http, format!("Error: {:?}", e))
                    .await,
            );
            return Ok(());
        }
    };

    enqueue(ctx, msg, input).await;
    handle_message(query_msg.delete(&ctx.http).await);

    Ok(())
}

async fn enqueue(ctx: &Context, msg: &Message, input: songbird::input::Input) {
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
            return;
        }
    };

    let manager = songbird::get(ctx).await.unwrap().clone();

    if manager.get(guild_id).is_none() {
        let (_, join_result) = manager.join(guild_id, channel_id).await;
        if let Err(e) = join_result {
            info!("Couldn't join voice channel: {:?}", e);
            handle_message(
                msg.channel_id
                    .say(&ctx, "Couldn't join voice channel: {:?}")
                    .await,
            );
            return;
        }
    }
    let meta = input.metadata.clone();
    #[cfg(feature = "cache")]
    let mut comp = None;
    #[cfg(feature = "cache")]
    let cache = {
        let read = ctx.data.read().await;
        read.get::<TrackCache>().unwrap().clone()
    };

    if let Some(_url) = meta.source_url {
        #[cfg(feature = "cache")]
        let input = if let Some(p) = cache.get(&_url).await.ok().flatten() {
            use songbird::input::dca;

            info!("Cache hit for {}", _url);

            let file = format!("audio_cache/{}", p);
            let mut input = dca(&file).await.unwrap();

            // Metadata that doesn't fit in the standard dca1 stuff is in the extra
            // field of the json metadata
            // TODO: remove from cache and fetch again if fail
            let extra_meta = {
                let mut reader = handle_io(File::open(&file).await);
                let mut header = [0u8; 4];

                handle_io(reader.read_exact(&mut header).await);

                if header != b"DCA1"[..] {
                    tracing::error!("Invalid magic bytes");
                    return;
                }

                let size = handle_io(reader.read_i32_le().await);
                if size < 2 {
                    tracing::error!("Invalid metadata size");
                    return;
                };

                let mut json = Vec::with_capacity(size as usize);
                let mut json_reader = reader.take(size as u64);

                handle_io(json_reader.read_to_end(&mut json).await);
                let value = serde_json::from_slice(&json).unwrap_or_default();
                cache::extra_meta(&value)
            };
            {
                input.metadata = Box::new(Metadata {
                    date: extra_meta.date,
                    duration: extra_meta.duration,
                    thumbnail: extra_meta.thumbnail,
                    ..*input.metadata
                })
            }
            input
        } else if let Some(d) = meta.duration {
            // TODO: Add config entry to limit lenght
            if d <= Duration::from_secs(1200) {
                match Compressed::new(input, Bitrate::BitsPerSecond(BITRATE as i32)) {
                    Ok(compressed) => {
                        comp = Some(compressed.new_handle());
                        // Load the whole thing into RAM.
                        // Audio artifacts appear when not doing this and loading the whole thing
                        // in ram is usually cheaper than keeping ytdl and ffmpeg open
                        let _ = compressed.raw.spawn_loader();
                        compressed.into()
                    }
                    Err(e) => {
                        warn!("Error creating compressed memory audio store: {:?}", e);
                        handle_message(
                            msg.channel_id
                                .say(&ctx.http, format!("Error: {:?}", e))
                                .await,
                        );
                        return;
                    }
                }
            } else {
                input
            }
        } else {
            input
        };

        // TODO: Add config entry to limit lenght
        #[cfg(not(feature = "cache"))]
        let input = if meta.duration <= Some(Duration::from_secs(1200)) {
            match Compressed::new(input, Bitrate::BitsPerSecond(128_000)) {
                Ok(compressed) => {
                    // Load the whole thing into RAM.
                    // Audio artifacts appear when not doing this and loading the whole thing
                    // in ram is usually cheaper than keeping ytdl and ffmpeg open
                    let _ = compressed.raw.spawn_loader();
                    compressed.into()
                }
                Err(e) => {
                    warn!("Error creating compressed memory audio store: {:?}", e);
                    handle_message(
                        msg.channel_id
                            .say(&ctx.http, format!("Error: {:?}", e))
                            .await,
                    );
                    return;
                }
            }
        } else {
            input
        };

        let manager = songbird::get(ctx).await.unwrap().clone();

        if manager.get(guild_id).is_none() {
            let (_, join_result) = manager.join(guild_id, channel_id).await;
            if let Err(_) = join_result {
                handle_message(
                    msg.channel_id
                        .say(&ctx, "Couldn't join voice channel")
                        .await,
                );
            }
        }

        let locked = manager.get(guild_id).unwrap();
        let mut call = locked.lock().await;

        let (track, track_handle) = songbird::tracks::create_player(input);

        let mut typemap = track_handle.typemap().write().await;
        typemap.insert::<TrackOwner>(msg.author.id);

        #[cfg(feature = "cache")]
        if let Some(c) = comp {
            let _ = track_handle.add_event(
                Event::Track(TrackEvent::End),
                TrackEndEvent {
                    cache: cache.clone(),
                    compressed: c,
                },
            );
        };

        call.enqueue(track);
    }
}
