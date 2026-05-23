#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("klyppd.sock")
}

fn main() {
    // Workarounds for webkit2gtk on Wayland — discovered through painful trial and error.
    // DMABUF renderer causes "Error 71 (Protocol error)" on Hyprland.
    // GTK overlay scrollbar is hideous and unstyleable from CSS.
    // Adwaita:dark gives us slightly less ugly fallback widgets.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1"); }
    }
    // Kill GTK scrollbar entirely — we use mousewheel/trackpad only
    unsafe {
        std::env::set_var("GTK_OVERLAY_SCROLLING", "0");
        std::env::set_var("GTK_THEME", "Adwaita:dark");
    }

    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, cmd, ..] = args.as_slice() {
        if flag == "--cmd" {
            send(cmd);
            return;
        }
    }
    klyppd_lib::run();
}

fn send(cmd: &str) {
    if let Ok(mut stream) = UnixStream::connect(socket_path()) {
        let _ = stream.write_all(cmd.as_bytes());
    } else {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "klyppd", "-u", "low", "-t", "1500", "klyppd", "App not running"])
            .status();
    }
}
