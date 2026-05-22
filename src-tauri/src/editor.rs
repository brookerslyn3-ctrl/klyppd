use std::process::Command;

type Error = Box<dyn std::error::Error>;

pub fn trim(input: &str, output: &str, start: f64, end: f64) -> Result<String, Error> {
    let ok = Command::new("ffmpeg")
        .args(["-y", "-ss", &format!("{start:.3}"), "-to", &format!("{end:.3}"), "-i", input, "-c", "copy", output])
        .status()?
        .success();
    if ok { Ok(output.into()) } else { Err("ffmpeg trim failed".into()) }
}

pub fn crop(input: &str, output: &str, x: u32, y: u32, w: u32, h: u32) -> Result<String, Error> {
    let filter = format!("crop={w}:{h}:{x}:{y}");
    let ok = Command::new("ffmpeg")
        .args(["-y", "-i", input, "-vf", &filter, "-c:v", "libx264", "-c:a", "copy", output])
        .status()?
        .success();
    if ok { Ok(output.into()) } else { Err("ffmpeg crop failed".into()) }
}

pub fn generate_thumbnail(input: &str, output: &str) -> Result<(), Error> {
    let ok = Command::new("ffmpeg")
        .args(["-y", "-ss", "1", "-i", input, "-vframes", "1", "-vf", "scale=320:-1", output])
        .status()?
        .success();
    if ok { Ok(()) } else { Err("thumbnail generation failed".into()) }
}

pub fn get_duration(input: &str) -> Result<f64, Error> {
    let out = Command::new("ffprobe")
        .args(["-v", "quiet", "-show_entries", "format=duration", "-of", "csv=p=0", input])
        .output()?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().parse()?)
}
