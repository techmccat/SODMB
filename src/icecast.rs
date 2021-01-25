use http::Uri;
use serde_json::Value;
use songbird::input::Metadata;

pub trait FromIceJson {
    fn from_ice_json(value: Value, uri: &str) -> Self;
}

impl FromIceJson for Metadata {
    fn from_ice_json(value: Value, query: &str) -> Self {
        let emptymeta = Metadata {
            title: None,
            artist: None,
            date: None,
            channels: None,
            start_time: None,
            duration: None,
            sample_rate: None,
            source_url: None,
            thumbnail: None,
        };

        let uri: Uri = query.parse().unwrap();
        let mount = uri.path();
        let obj = value.as_object().and_then(|o| o.get("icestats"));

        let title = obj
            .and_then(|m| m.get("host"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let artist = obj
            .and_then(|m| m.get("admin"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let source_val = {
            let list = obj.and_then(|m| m.get("source")).and_then(|v| match v {
                Value::Object(_) => Some(vec![v.to_owned()]),
                Value::Array(a) => Some(a.to_owned()),
                _ => None,
            });

            let mut found = None;

            if let Some(l) = list {
                for i in l {
                    if i.get("listenurl")
                        .and_then(|v| {
                            v.as_str()
                        })
                        .and_then(|u| {
                            u.rsplitn(2, "/").next()
                        }).and_then(|m| Some("/".to_owned() + m))
                        == Some(mount.to_owned())
                    {
                        found = Some(i);
                    }
                }
            }
            if let Some(source) = found {
                source
            } else {
                return emptymeta;
            }
        };
        let source = source_val.as_object();

        // TODO: Date is displayed with "/" in the middle for no reason
        let date = source
            .and_then(|m| m.get("stream_start"))
            .and_then(Value::as_str)
            .map(str::to_owned);

        let channels = source
            .and_then(|m| m.get("channels"))
            .and_then(Value::as_u64)
            .and_then(|i| Some(i as u8));

        let sample_rate = source
            .and_then(|m| m.get("samplerate"))
            .and_then(Value::as_u64)
            .and_then(|o| Some(o as u32));

        let thumbnail =
            Some("https://github.com/xiph/Icecast-Server/raw/master/web/icecast.png".to_owned());

        Metadata {
            title,
            artist,
            date,
            channels,
            start_time: None,
            duration: None,
            sample_rate,
            source_url: Some(query.to_owned()),
            thumbnail,
        }
    }
}
