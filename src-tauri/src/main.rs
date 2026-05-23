#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::Write;
use std::os::unix::net::UnixStream;

const SOCKET: &str = "/tmp/klyppd.sock";

fn main() {
    // Disable webkit2gtk's dmabuf renderer — broken on Hyprland and some Wayland setups.
    // Users can override by setting the env var themselves.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // SAFETY: single-threaded at this point (before Tauri spawns anything).
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
    if let Ok(mut stream) = UnixStream::connect(SOCKET) {
        let _ = stream.write_all(cmd.as_bytes());
    } else {
        let _ = std::process::Command::new("notify-send")
            .args(["-a", "klyppd", "-u", "low", "-t", "1500", "klyppd", "App not running"])
            .status();
    }
}
