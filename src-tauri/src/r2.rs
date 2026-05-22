use std::path::Path;
use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sdk_s3::config::{Builder, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;

use crate::db::Clip;
use crate::settings::AppSettings;

const ID_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const ID_LEN: usize = 6;

fn short_id() -> String {
    let mut n = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let mut id = String::with_capacity(ID_LEN);
    for _ in 0..ID_LEN {
        id.push(ID_CHARSET[(n % ID_CHARSET.len() as u64) as usize] as char);
        n = n.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    }
    id
}

fn client(s: &AppSettings) -> Client {
    let creds = Credentials::new(&s.r2_access_key, &s.r2_secret_key, None, None, "klyppd");
    let cfg = Builder::new()
        .region(Region::new("auto"))
        .endpoint_url(&s.r2_endpoint)
        .credentials_provider(creds)
        .force_path_style(true)
        .build();
    Client::from_conf(cfg)
}

fn fmt_bytes(b: u64) -> String {
    match b {
        0..=1023 => format!("{b} B"),
        1024..=1_048_575 => format!("{} KB", b / 1024),
        1_048_576..=1_073_741_823 => format!("{:.1} MB", b as f64 / 1_048_576.0),
        _ => format!("{:.2} GB", b as f64 / 1_073_741_824.0),
    }
}

/// "Sober_2026-05-22.mp4" → ("Sober", "2026-05-22")
fn split_filename(filename: &str) -> (String, String) {
    let stem = filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(filename);
    let bytes = stem.as_bytes();

    for (i, &b) in bytes.iter().enumerate() {
        if b != b'_' || i + 11 > bytes.len() { continue; }
        let candidate = &stem[i + 1..i + 11];
        if is_iso_date(candidate) {
            return (stem[..i].into(), candidate.into());
        }
    }
    (stem.into(), String::new())
}

fn is_iso_date(s: &str) -> bool {
    let b = s.as_bytes();
    s.len() == 10
        && b[4] == b'-' && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

fn pretty_date(date: &str) -> String {
    if date.len() < 10 { return date.into(); }
    const MONTHS: [&str; 12] = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    let month: usize = date[5..7].parse().unwrap_or(1);
    let day: usize = date[8..10].parse().unwrap_or(1);
    let year = &date[..4];
    if (1..=12).contains(&month) {
        format!("{} {}, {}", MONTHS[month - 1], day, year)
    } else {
        date.into()
    }
}

fn build_embed(video: &str, thumb: &str, title: &str, date: &str, size: &str, w: u32, h: u32) -> String {
    let site = format!("klyppd · {size}");
    let desc = if date.is_empty() {
        "Clipped with Klyppd".into()
    } else {
        format!("Clipped with Klyppd · {date}")
    };

    format!(r##"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<title>{title} — klyppd</title>
<meta property="og:type" content="video.other">
<meta property="og:title" content="{title}">
<meta property="og:description" content="{desc}">
<meta property="og:site_name" content="{site}">
<meta property="og:image" content="{thumb}">
<meta property="og:image:width" content="{w}">
<meta property="og:image:height" content="{h}">
<meta property="og:video" content="{video}">
<meta property="og:video:secure_url" content="{video}">
<meta property="og:video:type" content="video/mp4">
<meta property="og:video:width" content="{w}">
<meta property="og:video:height" content="{h}">
<meta name="twitter:card" content="player">
<meta name="twitter:title" content="{title}">
<meta name="twitter:description" content="{desc}">
<meta name="twitter:image" content="{thumb}">
<meta name="twitter:player" content="{video}">
<meta name="twitter:player:stream" content="{video}">
<meta name="twitter:player:stream:content_type" content="video/mp4">
<meta name="twitter:player:width" content="{w}">
<meta name="twitter:player:height" content="{h}">
<meta name="theme-color" content="#9accfa">
<style>
*{{box-sizing:border-box;margin:0;padding:0}}
body{{background:#0b0f12;color:#e0e2e8;font-family:-apple-system,'Inter','Segoe UI',sans-serif;display:flex;flex-direction:column;align-items:center;justify-content:center;min-height:100vh;padding:24px;font-size:14px}}
.head{{display:flex;align-items:baseline;gap:8px;margin-bottom:14px;font-size:12px;color:#8c9198}}
.brand{{color:#9accfa;font-weight:600}}
.size{{color:#42474e}}
.wrap{{max-width:1100px;width:100%}}
video{{width:100%;border-radius:8px;background:#000;display:block;box-shadow:0 8px 30px rgba(0,0,0,.5)}}
.title{{margin-top:16px;font-size:22px;font-weight:600;letter-spacing:-.01em}}
.sub{{margin-top:6px;font-size:13px;color:#8c9198}}
.foot{{margin-top:18px;padding-top:14px;border-top:1px solid #1c2024;font-size:11px;color:#42474e;display:flex;justify-content:space-between}}
.foot a{{color:#9accfa;text-decoration:none}}
</style></head>
<body>
<div class="wrap">
<div class="head"><span class="brand">klyppd</span><span class="size">{size}</span></div>
<video src="{video}" controls autoplay playsinline poster="{thumb}"></video>
<div class="title">{title}</div>
<div class="sub">{desc}</div>
<div class="foot"><span>{date}</span><span>Clipped with <a href="#">Klyppd</a></span></div>
</div>
</body></html>"##)
}

type R2Result<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub async fn upload(s: &AppSettings, clip: &Clip, permanent: bool) -> R2Result<String> {
    let c = client(s);
    let ext = Path::new(&clip.path).extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let id = short_id();
    let prefix = if permanent { "p" } else { "t" };
    let video_key = format!("{prefix}/{id}.{ext}");
    let thumb_key = format!("{prefix}/{id}.jpg");
    let html_key = format!("{prefix}/{id}");
    let domain = s.r2_custom_domain.trim_end_matches('/');

    let size = std::fs::metadata(&clip.path).map(|m| m.len()).unwrap_or(0);
    let (title, date) = split_filename(&clip.filename);

    c.put_object()
        .bucket(&s.r2_bucket)
        .key(&video_key)
        .body(ByteStream::from_path(&clip.path).await?)
        .content_type(format!("video/{ext}"))
        .send()
        .await?;

    let thumb_url = match clip.thumbnail_path.as_ref().filter(|p| Path::new(p).exists()) {
        Some(tp) => match ByteStream::from_path(tp).await {
            Ok(body) => {
                c.put_object()
                    .bucket(&s.r2_bucket)
                    .key(&thumb_key)
                    .body(body)
                    .content_type("image/jpeg")
                    .send()
                    .await?;
                format!("{domain}/{thumb_key}")
            }
            Err(_) => String::new(),
        },
        None => String::new(),
    };

    let video_url = format!("{domain}/{video_key}");
    let html = build_embed(&video_url, &thumb_url, &title, &pretty_date(&date), &fmt_bytes(size), 1280, 720);

    c.put_object()
        .bucket(&s.r2_bucket)
        .key(&html_key)
        .body(ByteStream::from(html.into_bytes()))
        .content_type("text/html; charset=utf-8")
        .send()
        .await?;

    Ok(format!("{domain}/{html_key}"))
}

pub async fn delete(s: &AppSettings, key: &str) -> R2Result<()> {
    let c = client(s);
    let base = strip_video_ext(key);
    let variants = [
        base.to_string(),
        format!("{base}.mp4"),
        format!("{base}.mkv"),
        format!("{base}.webm"),
        format!("{base}.jpg"),
    ];
    for v in &variants {
        let _ = c.delete_object().bucket(&s.r2_bucket).key(v).send().await;
    }
    Ok(())
}

pub async fn storage_usage(s: &AppSettings, prefix: &str) -> R2Result<u64> {
    let c = client(s);
    let mut total = 0u64;
    let mut continuation: Option<String> = None;

    loop {
        let mut req = c.list_objects_v2().bucket(&s.r2_bucket).prefix(prefix);
        if let Some(token) = &continuation {
            req = req.continuation_token(token);
        }
        let resp = req.send().await?;

        for obj in resp.contents() {
            total += obj.size().unwrap_or(0) as u64;
        }

        if !resp.is_truncated().unwrap_or(false) { break; }
        continuation = resp.next_continuation_token().map(String::from);
        if continuation.is_none() { break; }
    }

    Ok(total)
}

fn strip_video_ext(key: &str) -> &str {
    for ext in [".html", ".mp4", ".mkv", ".webm", ".jpg"] {
        if let Some(s) = key.strip_suffix(ext) { return s; }
    }
    key
}
