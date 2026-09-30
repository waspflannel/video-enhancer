use std::{io, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use serde_json::{json, Value};
use tao::{event::{Event, WindowEvent}, event_loop::{ControlFlow, EventLoopBuilder}, window::WindowBuilder, dpi::LogicalSize};
use video_enhancer::{job::VideoEnhancementJob, parser::{FileData, Parser}};
use crate::youtube::{self, DownloadOptions};
use crate::sample::{self, SampleClip, SampleSelection};
use wry::WebViewExtWindows;
use windows_core::{HSTRING, Interface};
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_3, COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY};

const PAGE_URL: &str = "https://enhancer.example/index.html";

enum AppEvent {
    Command(String),
    Loaded(Result<FileData, String>),
    Progress(Value),
    Finished(Result<Value, String>),
    Downloaded(Result<PathBuf, String>),
    SampleLoaded(Result<SampleClip, String>),
    SampleRendered(Result<PathBuf, String>),
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let preview_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tools/previews");
    std::fs::create_dir_all(&preview_root)?;
    let preview_workspace = tempfile::Builder::new().prefix("session-").tempdir_in(preview_root)?;
    let page = preview_workspace.path().join("index.html");
    std::fs::write(&page, include_str!("ui.html"))?;
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window = WindowBuilder::new().with_title("Video Enhancer").with_inner_size(LogicalSize::new(1120.0, 850.0)).with_min_inner_size(LogicalSize::new(780.0, 650.0)).build(&event_loop)?;
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();
    let webview = wry::WebViewBuilder::new()
        .with_ipc_handler(move |request| { let _ = ipc_proxy.send_event(AppEvent::Command(request.body().clone())); })
        .with_navigation_handler({
            let initial_page = AtomicBool::new(true);
            move |url| url == PAGE_URL && initial_page.swap(false, Ordering::Relaxed)
        })
        .build(&window)?;
    // Wry's IPC expects a URL with a host. WebView2 serves this folder locally,
    // including video seeking, without a server or access to other local files.
    // SAFETY: the controller and strings stay alive throughout these UI-thread calls.
    unsafe {
        let local_content: ICoreWebView2_3 = webview.controller().CoreWebView2()?.cast()?;
        local_content.SetVirtualHostNameToFolderMapping(&HSTRING::from("enhancer.example"), &HSTRING::from(std::path::absolute(preview_workspace.path())?.as_os_str()), COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY)?;
    }
    webview.load_url(PAGE_URL)?;
    let mut webview = Some(webview);
    let mut preview_workspace = Some(preview_workspace);
    let mut loaded_path: Option<PathBuf> = None;
    let mut sample_clip: Option<Arc<SampleClip>> = None;
    let mut sample_revision = 0_u64;
    let mut rendered_sample: Option<PathBuf> = None;
    let mut busy = false;
    let mut close_when_finished = false;
    let cancelled = Arc::new(AtomicBool::new(false));
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        let emit = |value: Value| { let _ = webview.as_ref().unwrap().evaluate_script(&format!("window.receive({value})")); };
        match event {
            Event::LoopDestroyed => {
                drop(webview.take());
                drop(sample_clip.take());
                drop(preview_workspace.take());
            }
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
                    "open-sample" if !busy => {
                        if let Some(path) = &rendered_sample && let Err(error) = std::path::absolute(path).and_then(|path| std::process::Command::new("explorer.exe").arg(path).spawn()) {
                            emit(json!({"type":"sample-error", "message":error.to_string()}));
                        }
                    }
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
                            sample_clip = None;
                            rendered_sample = None;
                            emit(json!({"type":"loading"}));
                            let sender = proxy.clone();
                            std::thread::spawn(move || {
                                let result = load_video_information(&path).map_err(|error| error.to_string());
                                let _ = sender.send_event(AppEvent::Loaded(result));
                            });
                        }
                    }
                    "load-sample" if !busy => {
                        let Some(input) = loaded_path.clone() else { return; };
                        let selection = match serde_json::from_value::<SampleSelection>(message["selection"].clone()) {
                            Ok(selection) => selection,
                            Err(error) => { emit(json!({"type":"sample-error", "message":error.to_string()})); return; }
                        };
                        busy = true;
                        sample_clip = None;
                        rendered_sample = None;
                        cancelled.store(false, Ordering::Relaxed);
                        emit(json!({"type":"sample-loading"}));
                        let workspace = preview_workspace.as_ref().unwrap().path().to_owned();
                        let sender = proxy.clone();
                        let cancellation = Arc::clone(&cancelled);
                        std::thread::spawn(move || {
                            let result = sample::load(&input, selection, &workspace, &cancellation).map_err(|error| error.to_string());
                            let _ = sender.send_event(AppEvent::SampleLoaded(result));
                        });
                    }
                    "render-sample" if !busy => {
                        let Some(clip) = sample_clip.clone() else { return; };
                        sample_revision += 1;
                        let mut value = message["job"].clone();
                        value["input"] = json!(clip.directory.path().join("source.mp4"));
                        value["output"] = json!(clip.directory.path().join(format!("enhanced-{sample_revision}.mp4")));
                        let job = match serde_json::from_value::<VideoEnhancementJob>(value) {
                            Ok(job) => job,
                            Err(error) => { emit(json!({"type":"sample-error", "message":error.to_string()})); return; }
                        };
                        busy = true;
                        cancelled.store(false, Ordering::Relaxed);
                        emit(json!({"type":"sample-rendering"}));
                        let sender = proxy.clone();
                        let cancellation = Arc::clone(&cancelled);
                        std::thread::spawn(move || {
                            let mut last_update = Instant::now();
                            let result = sample::render(job, &clip, &cancellation, |frames, _| {
                                if last_update.elapsed() >= Duration::from_millis(200) {
                                    let _ = sender.send_event(AppEvent::Progress(json!({"type":"sample-progress", "frames":frames})));
                                    last_update = Instant::now();
                                }
                            }).map_err(|error| error.to_string());
                            let _ = sender.send_event(AppEvent::SampleRendered(result));
                        });
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
            Event::UserEvent(AppEvent::SampleLoaded(result)) => {
                busy = false;
                match result {
                    Ok(clip) => {
                        match sample_url(&clip.directory.path().join("original.mp4"), preview_workspace.as_ref().unwrap().path()) {
                            Ok(url) => emit(json!({"type":"sample-loaded", "url":url, "start":clip.selection.start, "duration":clip.selection.duration})),
                            Err(error) => emit(json!({"type":"sample-error", "message":error.to_string()})),
                        }
                        sample_clip = Some(Arc::new(clip));
                    }
                    Err(error) => emit(json!({"type":"sample-error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::SampleRendered(result)) => {
                busy = false;
                match result.and_then(|path| { let url = sample_url(&path, preview_workspace.as_ref().unwrap().path()).map_err(|error| error.to_string())?; rendered_sample = Some(path); Ok(url) }) {
                    Ok(url) => emit(json!({"type":"sample-rendered", "url":url})),
                    Err(error) => emit(json!({"type":"sample-error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
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

fn sample_url(path: &Path, workspace: &Path) -> io::Result<String> {
    let relative = path.strip_prefix(workspace).map_err(io::Error::other)?;
    // Sample folders and filenames are generated by the app using URL-safe names.
    Ok(format!("https://enhancer.example/{}", relative.to_string_lossy().replace('\\', "/")))
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
