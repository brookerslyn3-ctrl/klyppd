mod db;
mod editor;
mod r2;
mod recorder;
mod settings;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use base64::Engine;
use chrono::{Duration, Local, Utc};
use serde_json::json;
use tauri::{Emitter, Manager};

use db::{Clip, Database};
use recorder::{Recorder, RecordingState};
use settings::AppSettings;

const PREVIEW_DIR: &str = "klyppd-preview";

fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

fn socket_path() -> PathBuf {
    runtime_dir().join("klyppd.sock")
}

fn pending_name_path() -> PathBuf {
    runtime_dir().join("klyppd-pending-name")
}

pub struct AppState {
    pub db: Mutex<Database>,
    pub recorder: Mutex<Recorder>,
    pub settings: Mutex<AppSettings>,
}

fn err<E: std::fmt::Display>(e: E) -> String { e.to_string() }

// Library queries -------------------------------------------------------------

#[tauri::command]
fn get_clips(state: tauri::State<AppState>) -> Result<Vec<Clip>, String> {
    state.db.lock().unwrap().get_all_clips().map_err(err)
}

#[tauri::command]
fn get_clips_by_folder(state: tauri::State<AppState>, folder: String) -> Result<Vec<Clip>, String> {
    state.db.lock().unwrap().get_clips_by_folder(&folder).map_err(err)
}

#[tauri::command]
fn get_uploaded_clips(state: tauri::State<AppState>, permanent: bool) -> Result<Vec<Clip>, String> {
    state.db.lock().unwrap().get_uploaded_clips(permanent).map_err(err)
}

#[tauri::command]
fn update_clip_tags(state: tauri::State<AppState>, id: String, tags: String) -> Result<(), String> {
    state.db.lock().unwrap().update_clip_tags(&id, &tags).map_err(err)
}

#[tauri::command]
fn update_clip_folder(state: tauri::State<AppState>, id: String, folder: String) -> Result<(), String> {
    state.db.lock().unwrap().update_clip_folder(&id, &folder).map_err(err)
}

#[tauri::command]
fn delete_clip(state: tauri::State<AppState>, id: String) -> Result<(), String> {
    state.db.lock().unwrap().delete_clip(&id).map_err(err)
}

#[tauri::command]
fn rename_clip(state: tauri::State<AppState>, id: String, new_name: String) -> Result<(), String> {
    let db = state.db.lock().unwrap();
    let clip = db.get_clip(&id).map_err(err)?;

    let old_path = Path::new(&clip.path);
    let ext = old_path.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let new_filename = if new_name.ends_with(&format!(".{ext}")) {
        new_name.clone()
    } else {
        format!("{new_name}.{ext}")
    };
    let new_path = old_path.parent().unwrap_or(Path::new(".")).join(&new_filename);

    std::fs::rename(old_path, &new_path).map_err(err)?;
    db.rename_clip(&id, &new_filename, &new_path.to_string_lossy()).map_err(err)
}

// Recording -------------------------------------------------------------------

#[tauri::command]
fn start_replay_buffer(state: tauri::State<AppState>) -> Result<(), String> {
    let s = state.settings.lock().unwrap().clone();
    state.recorder.lock().unwrap().start_replay_buffer(&s).map_err(err)
}

#[tauri::command]
fn stop_replay_buffer(state: tauri::State<AppState>) -> Result<(), String> {
    state.recorder.lock().unwrap().stop_replay_buffer().map_err(err)
}

#[tauri::command]
fn save_replay(state: tauri::State<AppState>) -> Result<(), String> {
    state.recorder.lock().unwrap().save_replay().map_err(err)
}

#[tauri::command]
fn start_recording(state: tauri::State<AppState>) -> Result<(), String> {
    let s = state.settings.lock().unwrap().clone();
    state.recorder.lock().unwrap().start_recording(&s).map_err(err)
}

#[tauri::command]
fn stop_recording(state: tauri::State<AppState>) -> Result<String, String> {
    state.recorder.lock().unwrap().stop_recording().map_err(err)
}

#[tauri::command]
fn get_recording_state(state: tauri::State<AppState>) -> RecordingState {
    state.recorder.lock().unwrap().get_state()
}

// Editor ----------------------------------------------------------------------

#[tauri::command]
async fn trim_clip(input: String, output: String, start: f64, end: f64) -> Result<String, String> {
    editor::trim(&input, &output, start, end).map_err(err)
}

#[tauri::command]
async fn crop_clip(input: String, output: String, x: u32, y: u32, w: u32, h: u32) -> Result<String, String> {
    editor::crop(&input, &output, x, y, w, h).map_err(err)
}

#[tauri::command]
async fn transcode_for_preview(input: String) -> Result<String, String> {
    // If it's already mp4 with aac audio, skip entirely — serve the original
    if input.ends_with(".mp4") && has_aac_audio(&input) {
        return Ok(input);
    }

    let dir = std::env::temp_dir().join(PREVIEW_DIR);
    std::fs::create_dir_all(&dir).map_err(err)?;
    let stem = Path::new(&input).file_stem().unwrap_or_default().to_string_lossy();
    let out = dir.join(format!("{stem}.mp4"));
    if out.exists() {
        return Ok(out.to_string_lossy().into());
    }

    let ok = std::process::Command::new("ffmpeg")
        .args(["-y", "-i", &input, "-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-movflags", "+faststart"])
        .arg(&out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !ok { return Err("ffmpeg transcode failed".into()); }
    Ok(out.to_string_lossy().into())
}

fn has_aac_audio(path: &str) -> bool {
    std::process::Command::new("ffprobe")
        .args(["-v", "quiet", "-select_streams", "a:0", "-show_entries", "stream=codec_name", "-of", "csv=p=0", path])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "aac")
        .unwrap_or(false)
}

// R2 -------------------------------------------------------------------------

#[tauri::command]
async fn upload_clip(state: tauri::State<'_, AppState>, id: String, permanent: bool) -> Result<String, String> {
    let s = state.settings.lock().unwrap().clone();
    let clip = state.db.lock().unwrap().get_clip(&id).map_err(err)?;
    let url = r2::upload(&s, &clip, permanent).await.map_err(err)?;
    let expiry = (!permanent).then(|| Utc::now() + Duration::days(s.expiry_days as i64));
    state.db.lock().unwrap().mark_uploaded(&id, &url, permanent, expiry).map_err(err)?;
    Ok(url)
}

#[tauri::command]
async fn delete_from_r2(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let s = state.settings.lock().unwrap().clone();
    let clip = state.db.lock().unwrap().get_clip(&id).map_err(err)?;
    let key = clip.r2_key.ok_or("clip not uploaded")?;
    r2::delete(&s, &key).await.map_err(err)?;
    state.db.lock().unwrap().mark_deleted(&id).map_err(err)
}

#[tauri::command]
async fn r2_storage(state: tauri::State<'_, AppState>, permanent: bool) -> Result<u64, String> {
    let s = state.settings.lock().unwrap().clone();
    if s.r2_bucket.is_empty() { return Ok(0); }
    let prefix = if permanent { "p/" } else { "t/" };
    r2::storage_usage(&s, prefix).await.map_err(err)
}

// Settings -------------------------------------------------------------------

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> AppSettings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_window_opacity(opacity: f64) -> Result<(), String> {
    let val = opacity.clamp(0.1, 1.0);

    if which_cmd("hyprctl") {
        let rule = format!("{val:.2} override {val:.2} override");
        std::process::Command::new("hyprctl")
            .args(["eval", &format!("hl.window_rule({{ match = {{ class = '^(klyppd)$' }}, opacity = '{rule}' }})")])
            .output()
            .ok();
    } else if which_cmd("swaymsg") {
        let pct = format!("{}", (val * 100.0) as u32);
        std::process::Command::new("swaymsg")
            .args(["[app_id=klyppd]", "opacity", &pct])
            .output()
            .ok();
    }
    Ok(())
}

fn which_cmd(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[tauri::command]
fn save_settings(state: tauri::State<AppState>, new_settings: AppSettings) -> Result<(), String> {
    *state.settings.lock().unwrap() = new_settings.clone();
    settings::save(&new_settings).map_err(err)
}

#[tauri::command]
fn get_storage_usage(state: tauri::State<AppState>) -> Result<u64, String> {
    let dir = state.settings.lock().unwrap().clips_directory.clone();
    Ok(std::fs::read_dir(&dir).map(|entries| {
        entries.flatten()
            .filter_map(|e| e.metadata().ok().map(|m| m.len()))
            .sum()
    }).unwrap_or(0))
}

#[tauri::command]
fn get_theme_css() -> Result<String, String> {
    let path = config_dir().join("theme.css");
    Ok(std::fs::read_to_string(path).unwrap_or_default())
}

// Dependency check -----------------------------------------------------------

#[tauri::command]
fn check_dependencies() -> Vec<String> {
    let mut missing = Vec::new();
    for dep in ["gpu-screen-recorder", "ffmpeg", "ffprobe"] {
        if !which_cmd(dep) {
            missing.push(dep.to_string());
        }
    }
    missing
}

// Files -----------------------------------------------------------------------

#[tauri::command]
fn read_thumbnail(path: String) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(err)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:image/jpeg;base64,{b64}"))
}

#[tauri::command]
fn read_video_bytes(path: String) -> Result<Vec<u8>, String> {
    std::fs::read(&path).map_err(err)
}

/// Serve a video file on a random localhost port, return the URL.
#[tauri::command]
fn serve_video(path: String) -> Result<String, String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::net::TcpListener;

    let file_len = std::fs::metadata(&path).map_err(err)?.len() as usize;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(err)?;
    let port = listener.local_addr().map_err(err)?.port();
    let url = format!("http://127.0.0.1:{port}/video.mp4");

    std::thread::spawn(move || {
        for _ in 0..20 {
            let Ok((mut stream, _)) = listener.accept() else { break; };
            let mut req = vec![0u8; 4096];
            let n = stream.read(&mut req).unwrap_or(0);
            let req_str = String::from_utf8_lossy(&req[..n]);

            let (start, end) = if let Some(range_line) = req_str.lines().find(|l| l.starts_with("Range:")) {
                let range = range_line.trim_start_matches("Range: bytes=");
                let parts: Vec<&str> = range.split('-').collect();
                let s: usize = parts.first().and_then(|p| p.parse().ok()).unwrap_or(0);
                let e: usize = parts.get(1).and_then(|p| if p.is_empty() { None } else { p.parse().ok() }).unwrap_or(file_len - 1);
                (s, e.min(file_len - 1))
            } else {
                (0, file_len - 1)
            };

            let chunk_len = end - start + 1;
            let header = if start == 0 && end == file_len - 1 {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {file_len}\r\nAccess-Control-Allow-Origin: *\r\nAccept-Ranges: bytes\r\n\r\n"
                )
            } else {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: video/mp4\r\nContent-Range: bytes {start}-{end}/{file_len}\r\nContent-Length: {chunk_len}\r\nAccess-Control-Allow-Origin: *\r\nAccept-Ranges: bytes\r\n\r\n"
                )
            };

            stream.write_all(header.as_bytes()).ok();

            let Ok(mut file) = std::fs::File::open(&path) else { break; };
            if file.seek(SeekFrom::Start(start as u64)).is_err() { continue; }
            let mut remaining = chunk_len;
            let mut buf = vec![0u8; 64 * 1024];
            while remaining > 0 {
                let to_read = remaining.min(buf.len());
                match file.read(&mut buf[..to_read]) {
                    Ok(0) => break,
                    Ok(n) => {
                        if stream.write_all(&buf[..n]).is_err() { break; }
                        remaining -= n;
                    }
                    Err(_) => break,
                }
            }
        }
    });

    Ok(url)
}

#[tauri::command]
fn replace_file(src: String, dst: String) -> Result<(), String> {
    std::fs::rename(&src, &dst).or_else(|_| {
        std::fs::copy(&src, &dst).map_err(err)?;
        std::fs::remove_file(&src).map_err(err)
    })
}

#[tauri::command]
fn scan_clips(state: tauri::State<AppState>) -> Result<Vec<Clip>, String> {
    let clips_dir = state.settings.lock().unwrap().clips_directory.clone();

    // Remove stale DB entries for clips whose files no longer exist on disk
    {
        let db = state.db.lock().unwrap();
        let all = db.get_all_clips().unwrap_or_default();
        for clip in &all {
            if !Path::new(&clip.path).exists() {
                db.delete_clip(&clip.id).ok();
            }
        }
    }

    let existing: std::collections::HashSet<String> = state.db.lock().unwrap()
        .get_all_clips()
        .unwrap_or_default()
        .into_iter()
        .map(|c| c.path)
        .collect();

    let thumb_dir = Path::new(&clips_dir).join(".thumbs");
    std::fs::create_dir_all(&thumb_dir).ok();

    let mut new_clips = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&clips_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !matches!(ext, "mkv" | "mp4" | "webm") { continue; }

            let path_str = path.to_string_lossy().to_string();
            if existing.contains(&path_str) { continue; }

            let created = entry.metadata().ok()
                .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                .map(|t| chrono::DateTime::<Utc>::from(t).to_rfc3339())
                .unwrap_or_else(|| Utc::now().to_rfc3339());

            let thumb = thumb_dir.join(format!("{}.jpg", uuid::Uuid::new_v4()));
            let thumb_path = editor::generate_thumbnail(&path_str, &thumb.to_string_lossy())
                .ok()
                .map(|_| thumb.to_string_lossy().to_string());

            new_clips.push(Clip {
                id: uuid::Uuid::new_v4().to_string(),
                filename: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
                path: path_str,
                duration: editor::get_duration(&path.to_string_lossy()).unwrap_or(0.0),
                created_at: created,
                thumbnail_path: thumb_path,
                tags: None,
                folder: None,
                upload_status: "local".into(),
                r2_key: None,
                r2_url: None,
                expiry_date: None,
                is_permanent: false,
            });
        }
    }

    if !new_clips.is_empty() {
        let db = state.db.lock().unwrap();
        for clip in &new_clips {
            db.insert_clip(clip).ok();
        }
    }

    state.db.lock().unwrap().get_all_clips().map_err(err)
}

// Helpers --------------------------------------------------------------------

fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("klyppd")
}

fn capture_window_class() -> String {
    if let Some(c) = try_hyprland_class() {
        return pretty_app_name(&c);
    }
    if let Some(c) = try_sway_class() {
        return pretty_app_name(&c);
    }
    if let Some(c) = try_gnome_class() {
        return pretty_app_name(&c);
    }
    if let Some(c) = try_kde_class() {
        return pretty_app_name(&c);
    }
    if let Some(c) = try_xdotool_class() {
        return pretty_app_name(&c);
    }
    "Clip".into()
}

fn try_hyprland_class() -> Option<String> {
    let out = std::process::Command::new("hyprctl")
        .args(["activewindow", "-j"])
        .output()
        .ok()?;
    extract_json_string(&String::from_utf8_lossy(&out.stdout), "class")
        .filter(|s| !s.is_empty())
}

fn try_sway_class() -> Option<String> {
    let out = std::process::Command::new("swaymsg")
        .args(["-t", "get_tree"])
        .output()
        .ok()?;
    let json = String::from_utf8_lossy(&out.stdout);
    extract_focused_app_id(&json)
}

fn try_gnome_class() -> Option<String> {
    let out = std::process::Command::new("gdbus")
        .args(["call", "--session", "--dest", "org.gnome.Shell",
               "--object-path", "/org/gnome/Shell",
               "--method", "org.gnome.Shell.Eval",
               "global.display.focus_window ? global.display.focus_window.get_wm_class() : ''"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let start = text.find('\'')? + 1;
    let end = text[start..].find('\'')?;
    let class = text[start..start + end].trim().to_string();
    if class.is_empty() { None } else { Some(class) }
}

fn try_kde_class() -> Option<String> {
    let out = std::process::Command::new("qdbus")
        .args(["org.kde.KWin", "/KWin", "org.kde.KWin.activeWindow"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { return None; }
    let out2 = std::process::Command::new("qdbus")
        .args(["org.kde.KWin", &format!("/Windows/{text}"), "org.kde.KWin.Window.resourceClass"])
        .output()
        .ok()?;
    let class = String::from_utf8_lossy(&out2.stdout).trim().to_string();
    if class.is_empty() { None } else { Some(class) }
}

fn try_xdotool_class() -> Option<String> {
    let out = std::process::Command::new("xdotool")
        .args(["getactivewindow", "getwindowclassname"])
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let class = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if class.is_empty() { None } else { Some(class) }
}

fn extract_focused_app_id(json: &str) -> Option<String> {
    let idx = json.find("\"focused\":true")?;
    let before = &json[..idx];
    let app_id = before.rfind("\"app_id\":\"")
        .map(|i| {
            let start = i + "\"app_id\":\"".len();
            let end = json[start..].find('"').unwrap_or(0);
            json[start..start + end].to_string()
        })
        .filter(|s| !s.is_empty() && s != "null");
    if app_id.is_some() { return app_id; }
    before.rfind("\"class\":\"")
        .map(|i| {
            let start = i + "\"class\":\"".len();
            let end = json[start..].find('"').unwrap_or(0);
            json[start..start + end].to_string()
        })
        .filter(|s| !s.is_empty() && s != "null")
}

fn extract_json_string(s: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":");
    let start = s.find(&needle)? + needle.len();
    let rest = &s[start..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')?;
    Some(rest[open..open + close].into())
}

/// Resolve a WM class to a human name via .desktop lookup.
/// `org.vinegarhq.Sober` → `Sober`, `com.spotify.Client` → `Spotify`.
fn pretty_app_name(class: &str) -> String {
    if let Some(name) = lookup_desktop_name(class) {
        return sanitize(&name);
    }

    let segments: Vec<&str> = class.rsplit('.').collect();
    for seg in &segments {
        let lower = seg.to_lowercase();
        if !matches!(lower.as_str(), "client" | "app" | "desktop" | "main") {
            return sanitize(seg);
        }
    }
    sanitize(class)
}

fn sanitize(s: &str) -> String {
    let cleaned: String = s.chars()
        .filter(|c| c.is_alphanumeric() || matches!(*c, '_' | '-'))
        .take(32)
        .collect();
    if cleaned.is_empty() { return "Clip".into(); }
    let mut iter = cleaned.chars();
    iter.next().unwrap().to_uppercase().chain(iter).collect()
}

fn lookup_desktop_name(class: &str) -> Option<String> {
    let mut app_dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
    ];
    if let Some(d) = dirs::data_dir() {
        app_dirs.push(d.join("applications"));
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_DIRS") {
        for dir in xdg.split(':') {
            let p = PathBuf::from(dir).join("applications");
            if !app_dirs.contains(&p) {
                app_dirs.push(p);
            }
        }
    }
    if let Some(home) = dirs::home_dir() {
        let flatpak = home.join(".local/share/flatpak/exports/share/applications");
        if !app_dirs.contains(&flatpak) {
            app_dirs.push(flatpak);
        }
    }
    let snap = PathBuf::from("/var/lib/snapd/desktop/applications");
    if !app_dirs.contains(&snap) {
        app_dirs.push(snap);
    }

    let target = class.to_lowercase();
    for dir in app_dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue; };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") { continue; }

            let Ok(text) = std::fs::read_to_string(&path) else { continue; };
            let stem_match = path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_lowercase() == target)
                .unwrap_or(false);

            let wm_match = text.lines()
                .filter_map(|l| l.strip_prefix("StartupWMClass="))
                .any(|v| v.trim().to_lowercase() == target);

            if !stem_match && !wm_match { continue; }

            if let Some(name) = text.lines().filter_map(|l| l.strip_prefix("Name=")).next() {
                return Some(name.trim().into());
            }
        }
    }
    None
}

fn notify_desktop(title: &str, body: &str) {
    let _ = std::process::Command::new("notify-send")
        .args(["-a", "klyppd", "-i", "video-x-generic", "-t", "2000", title, body])
        .status();
}

fn toast<R: tauri::Runtime>(handle: &tauri::AppHandle<R>, msg: &str, kind: &str) {
    let _ = handle.emit("toast", json!({ "msg": msg, "kind": kind }));
}

// Hotkey dispatcher (called via Unix socket from Hyprland binds) -------------

fn handle_hotkey(handle: &tauri::AppHandle, cmd: &str) {
    let state = handle.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();

    match cmd {
        "save-replay" => {
            let win = capture_window_class();
            let date = Local::now().format("%Y-%m-%d").to_string();
            std::fs::write(pending_name_path(), format!("{win}_{date}")).ok();

            match state.recorder.lock().unwrap().save_replay() {
                Ok(_) => {
                    let body = format!("Klyppd the last {}s of {}", settings.buffer_seconds, win);
                    toast(handle, &body, "ok");
                    notify_desktop("Klypp saved", &body);
                }
                Err(_) => {
                    toast(handle, "Buffer not running", "err");
                    notify_desktop("Klyppd", "Buffer not running");
                }
            }
        }
        "toggle-buffer" => {
            let mut rec = state.recorder.lock().unwrap();
            if rec.get_state().replay_buffer_active {
                let _ = rec.stop_replay_buffer();
                drop(rec);
                toast(handle, "Klyppd stopped", "ok");
                notify_desktop("Klyppd stopped", "Buffer is no longer recording");
            } else {
                let res = rec.start_replay_buffer(&settings);
                drop(rec);
                match res {
                    Ok(_) => {
                        let body = format!("Buffering last {}s", settings.buffer_seconds);
                        toast(handle, &body, "ok");
                        notify_desktop("Klyppd started", &body);
                    }
                    Err(_) => toast(handle, "Klyppd failed to start", "err"),
                }
            }
        }
        "toggle-recording" => {
            let mut rec = state.recorder.lock().unwrap();
            if rec.get_state().recording_active {
                let _ = rec.stop_recording();
                drop(rec);
                toast(handle, "Klypp recording stopped", "ok");
                notify_desktop("Klypp saved", "Recording stopped");
            } else {
                let res = rec.start_recording(&settings);
                drop(rec);
                match res {
                    Ok(_) => {
                        toast(handle, "Klypping…", "ok");
                        notify_desktop("Klypping…", "Press again to stop");
                    }
                    Err(_) => toast(handle, "Klypp failed", "err"),
                }
            }
        }
        _ => {}
    }
}

// Background workers ---------------------------------------------------------

fn spawn_tray(handle: tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show_item = MenuItem::with_id(&handle, "show", "Open klyppd", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(&handle, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(&handle, &[&show_item, &quit_item])?;

    let _ = TrayIconBuilder::with_id("klyppd-tray")
        .icon(handle.default_window_icon().cloned().unwrap())
        .tooltip("klyppd")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(&handle)?;

    Ok(())
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Reads /dev/input/ via evdev for global hotkeys — works on any compositor.
/// User needs to be in the `input` group: `sudo usermod -aG input $USER`
fn spawn_evdev_hotkeys(handle: tauri::AppHandle) {
    std::thread::spawn(move || {
        use evdev::{Device, InputEventKind, Key};
        use std::time::Instant;

        let grab_keyboards = || -> Vec<Device> {
            use std::os::unix::io::AsRawFd;
            evdev::enumerate()
                .filter_map(|(_, d)| {
                    if d.supported_keys().is_some_and(|k| k.contains(Key::KEY_A)) {
                        let fd = d.as_raw_fd();
                        unsafe {
                            let flags = libc::fcntl(fd, libc::F_GETFL);
                            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                        }
                        Some(d)
                    } else {
                        None
                    }
                })
                .collect()
        };

        let mut devices = grab_keyboards();

        if devices.is_empty() {
            eprintln!("klyppd: no keyboard found in /dev/input/ (are you in the 'input' group?)");
            return;
        }

        let mut last_clip = Instant::now() - std::time::Duration::from_secs(5);
        let mut last_rescan = Instant::now();

        loop {
            // Re-enumerate devices periodically to handle hotplug
            if last_rescan.elapsed() > std::time::Duration::from_secs(30) {
                last_rescan = Instant::now();
                let fresh = grab_keyboards();
                if !fresh.is_empty() {
                    devices = fresh;
                }
            }

            let mut had_events = false;
            let mut dead_indices = Vec::new();

            for (i, dev) in devices.iter_mut().enumerate() {
                match dev.fetch_events() {
                    Ok(events) => {
                        for ev in events {
                            had_events = true;
                            if let InputEventKind::Key(key) = ev.kind() {
                                update_modifiers(key, ev.value() != 0);

                                if ev.value() != 1 { continue; } // key down only

                                let state = handle.state::<AppState>();
                                let settings = state.settings.lock().unwrap().clone();

                                let matched = if hotkey_matches(&settings.hotkey_save_replay, key) {
                                    if last_clip.elapsed() > std::time::Duration::from_millis(1500) {
                                        last_clip = Instant::now();
                                        Some("save-replay")
                                    } else { None }
                                } else if hotkey_matches(&settings.hotkey_start_stop_recording, key) {
                                    Some("toggle-recording")
                                } else if hotkey_matches(&settings.hotkey_start_stop_buffer, key) {
                                    Some("toggle-buffer")
                                } else {
                                    None
                                };

                                if let Some(cmd) = matched {
                                    handle_hotkey(&handle, cmd);
                                }
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(_) => { dead_indices.push(i); }
                }
            }

            // Remove disconnected devices (iterate in reverse to preserve indices)
            for i in dead_indices.into_iter().rev() {
                devices.swap_remove(i);
            }

            if !had_events {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    });
}

/// Track modifier state globally for combo hotkeys
static MODS: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
const MOD_ALT: u8 = 1;
const MOD_CTRL: u8 = 2;
const MOD_SHIFT: u8 = 4;
const MOD_SUPER: u8 = 8;

fn update_modifiers(key: evdev::Key, down: bool) {
    use std::sync::atomic::Ordering;
    let bit = match key {
        evdev::Key::KEY_LEFTALT | evdev::Key::KEY_RIGHTALT => MOD_ALT,
        evdev::Key::KEY_LEFTCTRL | evdev::Key::KEY_RIGHTCTRL => MOD_CTRL,
        evdev::Key::KEY_LEFTSHIFT | evdev::Key::KEY_RIGHTSHIFT => MOD_SHIFT,
        evdev::Key::KEY_LEFTMETA | evdev::Key::KEY_RIGHTMETA => MOD_SUPER,
        _ => return,
    };
    if down {
        MODS.fetch_or(bit, Ordering::Relaxed);
    } else {
        MODS.fetch_and(!bit, Ordering::Relaxed);
    }
}

fn hotkey_matches(hotkey_str: &str, pressed: evdev::Key) -> bool {
    use std::sync::atomic::Ordering;

    let parts: Vec<&str> = hotkey_str.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() { return false; }

    let main_key = parts.last().unwrap().to_uppercase();
    let key_name = format!("KEY_{}", main_key);
    if format!("{:?}", pressed) != key_name { return false; }

    let mods = MODS.load(Ordering::Relaxed);
    let need_alt = parts.iter().any(|p| p.eq_ignore_ascii_case("alt"));
    let need_ctrl = parts.iter().any(|p| p.eq_ignore_ascii_case("ctrl"));
    let need_shift = parts.iter().any(|p| p.eq_ignore_ascii_case("shift"));
    let need_super = parts.iter().any(|p| p.eq_ignore_ascii_case("super") || p.eq_ignore_ascii_case("meta"));

    (need_alt == (mods & MOD_ALT != 0))
        && (need_ctrl == (mods & MOD_CTRL != 0))
        && (need_shift == (mods & MOD_SHIFT != 0))
        && (need_super == (mods & MOD_SUPER != 0))
}

fn spawn_socket_listener(handle: tauri::AppHandle) {
    use std::io::Read;
    use std::os::unix::net::UnixListener;

    std::thread::spawn(move || {
        let sock = socket_path();
        let _ = std::fs::remove_file(&sock);
        let Ok(listener) = UnixListener::bind(&sock) else { return; };

        for stream in listener.incoming().flatten() {
            let mut s = stream;
            let mut buf = String::new();
            if s.read_to_string(&mut buf).is_err() { continue; }
            handle_hotkey(&handle, buf.trim());
        }
    });
}

fn spawn_clips_watcher(handle: tauri::AppHandle, dir: String) {
    use std::collections::HashMap;
    use std::time::{Duration as StdDuration, Instant};
    use notify::{EventKind, RecursiveMode, Watcher};

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut watcher) = notify::recommended_watcher(tx) else { return; };
        if watcher.watch(Path::new(&dir), RecursiveMode::NonRecursive).is_err() { return; }

        let mut seen: HashMap<String, Instant> = HashMap::new();

        for event in rx.iter().flatten() {
            if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) { continue; }

            for path in event.paths {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if !matches!(ext, "mkv" | "mp4" | "webm") { continue; }

                let key = path.to_string_lossy().to_string();
                if seen.get(&key).is_some_and(|t| t.elapsed() < StdDuration::from_secs(60)) {
                    continue;
                }
                seen.insert(key, Instant::now());
                seen.retain(|_, t| t.elapsed() < StdDuration::from_secs(300));

                std::thread::sleep(StdDuration::from_millis(1200));

                let final_path = rename_if_pending(&path, ext).unwrap_or(path);
                let filename = final_path.file_name().unwrap_or_default().to_string_lossy().to_string();
                let _ = handle.emit("clip-saved", json!({
                    "filename": filename,
                    "path": final_path.to_string_lossy(),
                }));
            }
        }
    });
}

fn rename_if_pending(path: &Path, ext: &str) -> Option<PathBuf> {
    let pnp = pending_name_path();
    let pending = std::fs::read_to_string(&pnp).ok()?;
    let pending = pending.trim();
    if pending.is_empty() { return None; }

    let parent = path.parent().unwrap_or(Path::new("."));
    let mut target = parent.join(format!("{pending}.{ext}"));
    let mut n = 1;
    while target.exists() {
        target = parent.join(format!("{pending}_{n}.{ext}"));
        n += 1;
    }

    let result = std::fs::rename(path, &target).ok().map(|_| target);
    std::fs::remove_file(&pnp).ok();
    result
}

// Entry point ----------------------------------------------------------------

fn cleanup_preview_cache() {
    let dir = std::env::temp_dir().join(PREVIEW_DIR);
    if !dir.exists() { return; }
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(7 * 24 * 3600);
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let stale = entry.metadata().ok()
                .and_then(|m| m.modified().ok())
                .map(|t| t < cutoff)
                .unwrap_or(true);
            if stale {
                std::fs::remove_file(entry.path()).ok();
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::fs::create_dir_all(config_dir()).ok();
    cleanup_preview_cache();

    let data_dir = dirs::data_dir().unwrap_or_default().join("klyppd");
    std::fs::create_dir_all(&data_dir).ok();

    let settings = settings::load().unwrap_or_default();
    std::fs::create_dir_all(&settings.clips_directory).ok();
    let watch_dir = settings.clips_directory.clone();

    let db = Database::new(&data_dir.join("clips.db")).expect("open clips db");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(AppState {
            db: Mutex::new(db),
            recorder: Mutex::new(Recorder::new()),
            settings: Mutex::new(settings),
        })
        .setup(move |app| {
            spawn_socket_listener(app.handle().clone());
            spawn_clips_watcher(app.handle().clone(), watch_dir);
            spawn_tray(app.handle().clone())?;
            spawn_evdev_hotkeys(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_clips, get_clips_by_folder, get_uploaded_clips,
            update_clip_tags, update_clip_folder, delete_clip, rename_clip,
            start_replay_buffer, stop_replay_buffer, save_replay,
            start_recording, stop_recording, get_recording_state,
            trim_clip, crop_clip, transcode_for_preview,
            upload_clip, delete_from_r2, r2_storage,
            get_settings, save_settings, set_window_opacity, get_storage_usage, get_theme_css,
            check_dependencies,
            scan_clips, read_thumbnail, read_video_bytes, serve_video, replace_file,
        ])
        .run(tauri::generate_context!())
        .expect("run tauri app");
}
