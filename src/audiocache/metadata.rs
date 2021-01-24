use super::BITRATE;
use serde::{Deserialize, Serialize};
use songbird::input::Metadata;
use std::time::Duration;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DcaMetadata {
    dca: Dca,
    opus: Opus,
    info: Option<Info>,
    origin: Option<Origin>,
    extra: Extra,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Dca {
    version: u64,
    tool: Tool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Tool {
    name: String,
    version: String,
    url: String,
    author: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Opus {
    mode: String,
    sample_rate: u32,
    frame_size: u64,
    abr: u64,
    vbr: u64,
    channels: u8,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Info {
    title: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    genre: Option<String>,
    cover: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Origin {
    source: Option<String>,
    abr: Option<u64>,
    channels: Option<u8>,
    encoding: Option<String>,
    url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Extra {
    date: Option<String>,
    duration: Option<u64>,
    thumbnail: Option<String>,
}

impl From<Extra> for Metadata {
    fn from(d: Extra) -> Self {
        let (date, duration, thumbnail) =
            (d.date, d.duration.map(Duration::from_millis), d.thumbnail);
        Self {
            date,
            duration,
            thumbnail,
            ..Default::default()
        }
    }
}

impl From<Metadata> for DcaMetadata {
    fn from(m: Metadata) -> Self {
        let info = {
            if m.title.is_some() || m.artist.is_some() {
                Some(Info {
                    title: m.title,
                    artist: m.artist,
                    ..Default::default()
                })
            } else {
                None
            }
        };
        Self {
            dca: Dca {
                version: 1,
                tool: Tool {
                    name: "sodmb".to_owned(),
                    version: "0.1.0".to_owned(),
                    url: "https://github.com/techmccat/sodmb".to_owned(),
                    author: "me".to_owned(),
                },
            },
            opus: Opus {
                mode: "music".to_owned(),
                sample_rate: m.sample_rate.unwrap_or(48_000),
                frame_size: 960,
                abr: BITRATE,
                vbr: 1,
                channels: m.channels.unwrap(),
            },
            info,
            origin: Some(Origin {
                source: Some("file".to_owned()),
                url: m.source_url,
                ..Default::default()
            }),
            extra: Extra {
                date: m.date,
                duration: m.duration.and_then(|d| Some(d.as_millis() as u64)),
                thumbnail: m.thumbnail,
            },
        }
    }
}

impl DcaMetadata {
    pub fn header(&self) -> Vec<u8> {
        let json = serde_json::to_string(self).unwrap();
        let magic = b"DCA1";
        let len = (json.len() as i32).to_le_bytes();
        // there has to be a better way to do this
        magic
            .iter()
            .chain(len.iter())
            .chain(json.as_bytes())
            .map(|i| *i)
            .collect()
    }
}
