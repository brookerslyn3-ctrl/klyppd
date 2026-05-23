use std::process::{Child, Command};

use serde::{Deserialize, Serialize};

use crate::settings::AppSettings;

type Error = Box<dyn std::error::Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingState {
    pub replay_buffer_active: bool,
    pub recording_active: bool,
}

pub struct Recorder {
    replay: Option<Child>,
    recording: Option<Child>,
}

impl Recorder {
    pub fn new() -> Self {
        Self { replay: None, recording: None }
    }

    pub fn get_state(&self) -> RecordingState {
        RecordingState {
            replay_buffer_active: self.replay.is_some(),
            recording_active: self.recording.is_some(),
        }
    }

    pub fn start_replay_buffer(&mut self, s: &AppSettings) -> Result<(), Error> {
        if self.replay.is_some() {
            return Err("replay buffer already running".into());
        }

        let mut cmd = base_command(s);
        cmd.args(["-r", &s.buffer_seconds.to_string()])
           .args(["-c", &s.container])
           .args(["-o", &s.clips_directory]);

        self.replay = Some(cmd.spawn()?);
        Ok(())
    }

    pub fn stop_replay_buffer(&mut self) -> Result<(), Error> {
        let mut child = self.replay.take().ok_or("no replay buffer running")?;
        send_signal(&child, libc::SIGINT);
        child.wait()?;
        Ok(())
    }

    pub fn save_replay(&mut self) -> Result<(), Error> {
        let child = self.replay.as_ref().ok_or("no replay buffer running")?;
        send_signal(child, libc::SIGUSR1);
        Ok(())
    }

    pub fn start_recording(&mut self, s: &AppSettings) -> Result<(), Error> {
        if self.recording.is_some() {
            return Err("already recording".into());
        }

        let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let output = format!("{}/{}.{}", s.clips_directory, stamp, s.container);

        let mut cmd = base_command(s);
        cmd.args(["-o", &output]);

        self.recording = Some(cmd.spawn()?);
        Ok(())
    }

    pub fn stop_recording(&mut self) -> Result<String, Error> {
        let mut child = self.recording.take().ok_or("not recording")?;
        send_signal(&child, libc::SIGINT);
        child.wait()?;
        Ok("recording saved".into())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        for child in [self.replay.as_mut(), self.recording.as_mut()].into_iter().flatten() {
            send_signal(child, libc::SIGINT);
            child.wait().ok();
        }
    }
}

fn base_command(s: &AppSettings) -> Command {
    let mut cmd = Command::new("gpu-screen-recorder");
    cmd.args(["-w", "portal"])
       .args(["-f", &s.fps.to_string()])
       .args(["-k", &s.codec])
       .args(["-ac", &s.audio_codec]);

    if !s.audio_source.is_empty() {
        cmd.args(["-a", &s.audio_source]);
    }
    cmd
}

fn send_signal(child: &Child, signal: i32) {
    unsafe { libc::kill(child.id() as i32, signal); }
}
