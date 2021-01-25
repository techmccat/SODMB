use super::utils::*;
use serde_json::Value;
use serenity::{
    client::Context,
    framework::standard::{macros::command, Args, CommandResult},
    model::channel::Message,
};
use songbird::input::Metadata;

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
