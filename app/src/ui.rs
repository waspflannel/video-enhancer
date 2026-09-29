use std::{io, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use serde_json::{json, Value};
use tao::{event::{Event, WindowEvent}, event_loop::{ControlFlow, EventLoopBuilder}, window::WindowBuilder, dpi::LogicalSize};
use video_enhancer::{job::{VideoEnhancementJob, EnhancementSettings}, parser::{FileData, Parser}};
use crate::youtube::{self, DownloadOptions};

enum AppEvent {
    Command(String),
    Loaded(Result<FileData, String>),
    Progress(Value),
    Finished(Result<Value, String>),
    Downloaded(Result<PathBuf, String>),
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window = WindowBuilder::new().with_title("Video Enhancer").with_inner_size(LogicalSize::new(1120.0, 850.0)).with_min_inner_size(LogicalSize::new(780.0, 650.0)).build(&event_loop)?;
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();
    let webview = wry::WebViewBuilder::new()
        .with_html(include_str!("ui.html"))
        .with_ipc_handler(move |request| { let _ = ipc_proxy.send_event(AppEvent::Command(request.body().clone())); })
        .with_navigation_handler({
            // Wry loads the bundled HTML as a data URL. Block subsequent navigation.
            let initial_page = AtomicBool::new(true);
            move |url| url.starts_with("data:text/html") && initial_page.swap(false, Ordering::Relaxed)
        })
        .build(&window)?;
    let mut loaded_path: Option<PathBuf> = None;
    let mut busy = false;
    let mut close_when_finished = false;
    let cancelled = Arc::new(AtomicBool::new(false));
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let emit = |value: Value| { let _ = webview.evaluate_script(&format!("window.receive({value})")); };
        match event {
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                if busy {
                    close_when_finished = true;
                    cancelled.store(true, Ordering::Relaxed);
                    emit(json!({"type":"status", "message":"Stopping before closing…"}));
                } else { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::Command(body)) => {
                let Ok(message) = serde_json::from_str::<Value>(&body) else { return; };
                match message["command"].as_str().unwrap_or("") {
                    "ready" => emit(json!({"type":"presets", "presets":EnhancementSettings::presets()})),
                    "cancel" => { cancelled.store(true, Ordering::Relaxed); }
                    "choose-hdri" if !busy => {
                        if let Some(path) = rfd::FileDialog::new().set_title("Choose a relighting environment").add_filter("HDR environment", &["hdr", "exr", "pfm"]).pick_file() {
                            emit(json!({"type":"hdri-selected", "path":path, "name":path.file_name().unwrap_or_default().to_string_lossy()}));
                        }
                    }
                    "download" if !busy => {
                        let options = match serde_json::from_value::<DownloadOptions>(message["options"].clone()) {
                            Ok(options) => options,
                            Err(error) => { emit(json!({"type":"download-error", "message":error.to_string()})); return; }
                        };
                        busy = true;
                        cancelled.store(false, Ordering::Relaxed);
                        emit(json!({"type":"download-started"}));
                        let sender = proxy.clone();
                        let cancellation = Arc::clone(&cancelled);
                        std::thread::spawn(move || {
                            let result = youtube::download(options, &cancellation, |message| {
                                let _ = sender.send_event(AppEvent::Progress(json!({"type":"download-progress", "message":message})));
                            }).map_err(|error| error.to_string());
                            let _ = sender.send_event(AppEvent::Downloaded(result));
                        });
                    }
                    "load" if !busy => {
                        if let Some(path) = rfd::FileDialog::new().set_title("Load video").set_directory(youtube::directory()).add_filter("Video", &["mp4", "mkv", "mov", "avi", "webm", "m4v"]).pick_file() {
                            busy = true;
                            loaded_path = None;
                            emit(json!({"type":"loading"}));
                            let sender = proxy.clone();
                            std::thread::spawn(move || {
                                let result = load_video_information(&path).map_err(|error| error.to_string());
                                let _ = sender.send_event(AppEvent::Loaded(result));
                            });
                        }
                    }
                    "generate" if !busy => {
                        let Some(source_path) = loaded_path.as_ref() else { return; };
                        let name = format!("{}-enhanced.mp4", source_path.file_stem().unwrap_or_default().to_string_lossy());
                        let Some(output) = rfd::FileDialog::new().set_title("Save enhanced video — choose a new filename").add_filter("MP4 video", &["mp4"]).set_file_name(name).save_file() else { return; };
                        let mut value = message["job"].clone();
                        value["input"] = json!(source_path);
                        value["output"] = json!(output);
                        let job = match serde_json::from_value::<VideoEnhancementJob>(value) {
                            Ok(job) => job,
                            Err(error) => { emit(json!({"type":"error", "message":error.to_string()})); return; }
                        };
                        busy = true;
                        cancelled.store(false, Ordering::Relaxed);
                        emit(json!({"type":"started", "output":output}));
                        let sender = proxy.clone();
                        let cancellation = Arc::clone(&cancelled);
                        std::thread::spawn(move || {
                            let started = Instant::now();
                            let mut last_update = Instant::now();
                            let result = job.run(&cancellation, |frames, seconds| {
                                if last_update.elapsed() >= Duration::from_millis(200) {
                                    let _ = sender.send_event(AppEvent::Progress(json!({"type":"progress", "frames":frames, "seconds":seconds, "elapsed":started.elapsed().as_secs_f64()})));
                                    last_update = Instant::now();
                                }
                            }).map(|frames| json!({"type":"finished", "frames":frames, "elapsed":started.elapsed().as_secs_f64(), "output":job.output})).map_err(|error| error.to_string());
                            let _ = sender.send_event(AppEvent::Finished(result));
                        });
                    }
                    _ => {}
                }
            }
            Event::UserEvent(AppEvent::Loaded(result)) => {
                busy = false;
                match result {
                    Ok(source) => {
                        loaded_path = Some(source.path.clone());
                        emit(json!({"type":"loaded", "source":{"name":source.path.file_name().unwrap_or_default().to_string_lossy(), "width":source.width, "height":source.height, "fps":source.fps, "duration":source.duration_seconds, "audio":source.audio_streams.len(), "pixel_format":source.pixel_format}}));
                    }
                    Err(error) => emit(json!({"type":"error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::Progress(value)) => emit(value),
            Event::UserEvent(AppEvent::Downloaded(result)) => {
                busy = false;
                match result {
                    Ok(path) => emit(json!({"type":"downloaded", "path":path})),
                    Err(error) => emit(json!({"type":"download-error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::Finished(result)) => {
                busy = false;
                emit(result.unwrap_or_else(|error| json!({"type":"error", "message":error})));
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            _ => {}
        }
    });
}

fn load_video_information(path: &Path) -> io::Result<FileData> {
    let source = Parser::new(path).get_video_information()?;
    if !["h264", "hevc", "av1"].contains(&source.codec.as_str()) || !["yuv420p", "yuvj420p", "nv12", "yuv420p10le", "p010le"].contains(&source.pixel_format.as_str()) {
        return Err(io::Error::other("Choose an 8-bit or 10-bit SDR H.264, HEVC or AV1 video."));
    }
    let video_stream = source.metadata["streams"].as_array().and_then(|streams| streams.iter().find(|stream| stream["index"].as_u64() == Some(u64::from(source.video_stream_index))));
    if video_stream.and_then(|stream| stream["color_transfer"].as_str()).is_some_and(|transfer| ["smpte2084", "arib-std-b67"].contains(&transfer)) {
        return Err(io::Error::other("Choose an SDR source. TrueHDR converts SDR to HDR10; existing HDR sources are not supported."));
    }
    Ok(source)
}
