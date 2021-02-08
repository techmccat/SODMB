use crate::commands::utils::handle_io;
use http::Uri;
use serde_json::Value;
use serenity::async_trait;
use songbird::{
    input::{cached::Compressed, Metadata},
    Event, EventContext, EventHandler,
};
use sqlx::{any::AnyConnection, Connection, Executor, Row};
use std::{fs, path::Path, sync::Arc, time::Duration};
use tokio::{fs::File, io::AsyncWriteExt, sync::Mutex};
use tracing::{info, warn};

pub const BITRATE: u64 = 128_000;

mod metadata;

type DbResult<T> = Result<T, sqlx::Error>;

#[derive(Debug, Clone)]
pub struct TrackCache {
    pub connection: Arc<Mutex<AnyConnection>>,
}

#[derive(Debug)]
struct CacheRow {
    uri: String,
    path: String,
}

#[derive(Debug)]
pub struct TrackEndEvent {
    pub cache: TrackCache,
    pub compressed: Compressed,
}

impl TrackCache {
    pub async fn new(uri: &str) -> DbResult<TrackCache> {
        let mut conn = AnyConnection::connect(uri).await?;
        conn.execute("BEGIN").await?;
        Ok(TrackCache {
            connection: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn get(&self, uri: &str) -> DbResult<Option<String>> {
        let mut conn = self.connection.lock().await;

        let row = sqlx::query(&format!(
            "
select Path from Cache
where Uri=\"{}\"
            ",
            uri
        ))
        .fetch_optional(&mut *conn)
        .await?;

        Ok(row.map(|r| r.get("Path")).flatten())
    }

    async fn insert(&self, row: CacheRow) -> DbResult<Option<i64>> {
        let mut conn = self.connection.lock().await;

        let res = sqlx::query(&format!(
            "
insert into Cache values('{}', '{}')
            ",
            row.uri, row.path
        ))
        .execute(&mut *conn)
        .await?;

        // Use with Any*
        Ok(res.last_insert_id())
        //Ok(Some(res.last_insert_rowid()))
    }
}

#[async_trait]
impl EventHandler for TrackEndEvent {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(_) = ctx {
            let meta = self.compressed.metadata.clone();

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

                    // AsyncRead is a mess I dont' really want to deal with ATM.
                    // Take a look at the traits for TxCatcher and feel my pain
                    size += handle_io(
                        tokio::task::spawn_blocking(move || {
                            std::io::copy(&mut comp_send, &mut send_file)
                        })
                        .await
                        .unwrap(),
                    );
                    info!("Wrote {}KiB", size / 1024);

                    if let Err(e) = self
                        .cache
                        .insert(CacheRow {
                            uri: sauce,
                            path: format!("{}/{}", host, query),
                        })
                        .await
                    {
                        warn!("Error adding entry to cache: {}", e)
                    }
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
