use commands::*;
use serde::Deserialize;
use serenity::{
    async_trait,
    client::{bridge::gateway::ShardManager, Client, Context, EventHandler},
    framework::{standard::macros::group, StandardFramework},
    model::gateway::{Activity, Ready},
    prelude::TypeMapKey,
};
use songbird::SerenityInit;
use std::{collections::HashMap, env, fs, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tracing::warn;

#[cfg(feature = "cache")]
mod cache;

mod commands;
mod icecast;

#[derive(Deserialize)]
struct Config {
    token: String,
    prefix: String,
}

struct Handler {
    prefix: String,
}

#[group]
#[commands(stats)]
struct Misc;

#[group]
#[commands(
    add, raw, icecast, pause, play, skip, clear, queue, pop, leave, join, np
)]
struct Music;

struct ShardManagerContainer;
struct CommandCounter;

#[derive(Clone)]
struct QueueEntry {
    url: String,
    file: Option<PathBuf>,
    owner: u64,
}

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

impl TypeMapKey for CommandCounter {
    type Value = HashMap<String, u64>;
}

#[cfg(feature = "cache")]
use cache::TrackCache;
#[cfg(feature = "cache")]
impl TypeMapKey for TrackCache {
    type Value = TrackCache;
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        warn!("{} connected", ready.user.name);
        let act = format!("{}help", self.prefix);
        ctx.set_activity(Activity::playing(&act)).await;
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let config = read_config()?;

    let framework = StandardFramework::new()
        .configure(|c| c.prefix(&config.prefix))
        .group(&MUSIC_GROUP)
        .group(&MISC_GROUP)
        .before(before)
        .after(after)
        .help(&HELP);

    let mut client = Client::builder(config.token)
        .event_handler(Handler {
            prefix: config.prefix,
        })
        .framework(framework)
        .register_songbird()
        .await
        .unwrap();

    {
        let mut data = client.data.write().await;
        data.insert::<CommandCounter>(HashMap::default());
        data.insert::<ShardManagerContainer>(Arc::clone(&client.shard_manager));

        #[cfg(feature = "cache")]
        match TrackCache::new("sqlite://audio_cache/cache.db").await {
            Ok(tc) => data.insert::<TrackCache>(tc),
            Err(e) => tracing::error!(
                "Database connection error: {}
Cache will be disabled",
                e
            ),
        }
    }

    let shard_manager = client.shard_manager.clone();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.expect("Could not get signal");
        warn!("Shutting down shards");
        shard_manager.lock().await.shutdown_all().await;
    });

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let shard_manager = client.shard_manager.clone();

        tokio::spawn(async move {
            let mut signal = signal(SignalKind::terminate()).unwrap();
            signal.recv().await;
            warn!("Shutting down shards");
            shard_manager.lock().await.shutdown_all().await;
        });
    }

    client.start().await?;
    Ok(())
}

fn read_config() -> Result<Config, Box<dyn std::error::Error>> {
    Ok(toml::from_str({
        &fs::read_to_string(env::current_exe()?.parent().unwrap().join("config.toml"))
            .unwrap_or(fs::read_to_string(env::current_dir()?.join("config.toml"))?)
    })?)
}
