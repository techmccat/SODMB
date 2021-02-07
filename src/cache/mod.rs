use crate::commands::utils::handle_io;
use http::Uri;
use serde_json::Value;
use serenity::async_trait;
use songbird::{
    input::{cached::Compressed, Metadata},
    Event, EventContext, EventHandler,
};
use std::{collections::HashMap, fs, path::Path, sync::Arc, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt, sync::Mutex};
use tracing::info;

pub const BITRATE: u64 = 128_000;

mod metadata;

#[derive(Debug, Default)]
pub struct TrackCache(pub HashMap<String, String>);

#[derive(Debug)]
pub struct TrackEndEvent {
    pub cache: Arc<Mutex<TrackCache>>,
    pub compressed: Compressed,
}

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
        info!("Wrote JSON at {}", cold)
    }
}

#[async_trait]
impl EventHandler for TrackEndEvent {
    //#[instrument(skip(ctx))]
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(track_list) = ctx {
            let (_, handle) = track_list.last().unwrap();
            let meta = self.compressed.metadata.clone();
            if self
                .cache
                .lock()
                .await
                .0
                .get(&handle.metadata().source_url.clone().unwrap())
                .is_some()
            {
                info!("Already cached");
                return None;
            }
            // only cache if shorter than 20min
            if let Some(d) = meta.duration {
                if d <= Duration::from_secs(1200) {
                    info!("Starting cache write");
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

                    let path = format!("audio_cache/{}", host);
                    if !Path::new(&path).exists() {
                        handle_io(fs::create_dir_all(&path));
                    };
                    let path = format!("{}/{}", path, query);
                    let mut file = handle_io(File::create(&path).await);

                    let mut size = handle_io(file.write(&dcameta.header()).await) as u64;

                    let mut send_file = file.into_std().await;
                    let mut comp_send = self.compressed.raw.new_handle();

                    // AsyncRead is a clusterfuck i dont' really want to deal with it ATM.
                    // Take a look at the traits for TxCatcher and feel my pain
                    size += handle_io(
                        tokio::task::spawn_blocking(move || {
                            std::io::copy(&mut comp_send, &mut send_file)
                        })
                        .await
                        .unwrap(),
                    );
                    info!("Wrote {}KB", size / 1024);

                    let mut lock = self.cache.lock().await;
                    lock.0.insert(sauce, format!("{}/{}", host, query));
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
