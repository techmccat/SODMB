use crate::commands::{utils::handle_io, CompHandle};
use http::Uri;
use serde_json::Value;
use serenity::async_trait;
use songbird::{
    input::{cached::Compressed, Metadata},
    Event, EventContext, EventHandler,
};
use std::{collections::HashMap, fs, io::Read, path::Path, sync::Arc, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt, sync::Mutex};

pub const BITRATE: u64 = 128_000;

mod metadata;

#[derive(Default)]
pub struct TrackCache(pub HashMap<String, String>);

pub struct TrackEndEvent {
    pub cache: Arc<Mutex<TrackCache>>,
    pub compressed: Compressed,
}

pub struct TrackStartEvent;

impl TrackCache {
    pub fn new() -> Self {
        let buf = fs::read_to_string("audio_cache/cold.json").unwrap_or_default();
        Self {
            0: serde_json::from_str(&buf).unwrap_or_default(),
        }
    }
}

// TODO: Flush this once in a while
impl Drop for TrackCache {
    fn drop(&mut self) {
        let cold = "audio_cache/cold.json";
        if !Path::new("audio_cache").exists() {
            handle_io(fs::create_dir("audio_cache"));
        };
        handle_io(fs::write(
            cold,
            &serde_json::to_string(&self.0).unwrap().into_bytes(),
        ));
    }
}

#[async_trait]
impl EventHandler for TrackEndEvent {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(track_list) = ctx {
            let (_, handle) = track_list.last().unwrap();
            let meta = self.compressed.metadata.clone();
            // if already cached, do nothing
            if self
                .cache
                .lock()
                .await
                .0
                .get(&handle.metadata().source_url.clone().unwrap())
                .is_some()
            {
                return None;
            }

            let duration = meta.duration.unwrap();
            let time = duration.as_secs();
            let len = (time + 1) * BITRATE / 8;

            // saves file as audio_cache/host/query
            let sauce = meta.source_url.clone().unwrap();
            let (query, host) = {
                let uri = sauce.parse::<Uri>().unwrap();
                (
                    uri.query().unwrap().to_owned(),
                    uri.host().unwrap().to_owned(),
                )
            };
            // songbird doesn't output dca1, so I'll do it myself
            let dcameta = metadata::DcaMetadata::from(meta.clone());

            let mut comp_send = self.compressed.raw.new_handle();
            let dca = tokio::task::spawn_blocking(move || {
                // println!("Preallocating {} bytes", len);
                let mut buf = Vec::with_capacity(len as usize);
                // println!("Read {} bytes",
                handle_io(comp_send.read_to_end(&mut buf));
                buf
            })
            .await
            .unwrap();

            {
                let path = format!("audio_cache/{}", host);
                if !Path::new(&path).exists() {
                    handle_io(fs::create_dir_all(&path));
                };
                let path = format!("{}/{}", path, query);
                let mut file = handle_io(File::create(&path).await);
                // TODO: tracing
                // println!("Writing to {}", path);
                // println!(
                //    "Header: {} bytes",
                handle_io(file.write(&dcameta.header()).await);
                //);
                handle_io(file.write_all(&dca).await);
            }
            let mut lock = self.cache.lock().await;
            lock.0.insert(sauce, format!("{}/{}", host, query));
        }
        None
    }
}

// There's no event that fires when a track plays, so i'll just
// preload the next track when the previous ends
#[async_trait]
impl EventHandler for TrackStartEvent {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(track_list) = ctx {
            // Get next track in queue
            if let Some((_, handle)) = track_list.get(1) {
                // If there's a handle to the Opus encoder, load the reader
                let read = handle.typemap().read().await;
                if let Some(comp) = read.get::<CompHandle>() {
                    comp.raw.spawn_loader();
                }
            }
        }
        None
    }
}

pub fn extra_meta(val: &Value) -> Metadata {
    let obj = if let Some(o) = val.as_object().and_then(|o| o.get("extra")) {
        o
    } else {
        return Metadata::default();
    };
    Metadata {
        date: obj.get("date").and_then(Value::as_str).map(str::to_owned),
        duration: obj
            .get("duration")
            .and_then(Value::as_u64)
            .map(Duration::from_millis),
        thumbnail: obj
            .get("thumbnail")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Default::default()
    }
}
