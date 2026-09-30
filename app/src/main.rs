#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod ui;
mod youtube;
mod sample;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The same job can be replayed without opening the desktop UI.
    if let Some(path) = std::env::args_os().nth(1) {
        let job: video_enhancer::job::VideoEnhancementJob = serde_json::from_slice(&std::fs::read(path)?)?;
        let count = job.run(&std::sync::atomic::AtomicBool::new(false), |_, _| {})?;
        println!("Saved {count} frames to {}", job.output.display());
        return Ok(());
    }
    ui::run()
}
