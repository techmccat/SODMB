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

impl TrackCache {
    pub fn new() -> Self {
        let buf = fs::read_to_string("audio_cache/cold.json").unwrap_or("{}".to_owned());
        Self {
            0: serde_json::from_str(&buf).unwrap(),
        }
    }
}

impl Drop for TrackCache {
    fn drop(&mut self) {
        let cold = "audio_cache/cold.json";
        if !Path::new("audio_cache").exists() {
            fs::create_dir("audio_cache").unwrap();
        };
        fs::write(cold, &serde_json::to_string(&self.0).unwrap().into_bytes()).unwrap();
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
            // only cache if shorter than 20min
            if let Some(d) = meta.duration {
                if d <= Duration::from_secs(1200) {
                    let time = d.as_secs();
                    let len = (time + 1) * BITRATE / 8;
                    // saves file as audio_cache/host/query
                    let (query, host) = {
                        let uri = meta.source_url.clone().unwrap().parse::<Uri>().unwrap();
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
                        comp_send.read_to_end(&mut buf).unwrap();
                        buf
                    })
                    .await
                    .unwrap();

                    {
                        let path = format!("audio_cache/{}", host);
                        if !Path::new(&path).exists() {
                            fs::create_dir_all(&path).unwrap();
                        };
                        let path = format!("{}/{}", path, query);
                        let mut file = File::create(&path).await.unwrap();
                        println!("Writing to {}", path);
                        println!(
                            "Header: {} bytes",
                            file.write(&dcameta.header()).await.unwrap()
                        );
                        file.write_all(&dca).await.unwrap();
                    }
                    let mut lock = self.cache.lock().await;
                    lock.0.insert(
                        meta.clone().source_url.unwrap(),
                        format!("{}/{}", host, query),
                    );
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
