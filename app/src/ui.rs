use std::{collections::HashMap, io::{self, Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};
use serde_json::{json, Value};
use tao::{event::{Event, WindowEvent}, event_loop::{ControlFlow, EventLoopBuilder}, window::WindowBuilder, dpi::LogicalSize};
use video_enhancer::{job::VideoEnhancementJob, parser::{FileData, Parser}};
use crate::youtube::{self, DownloadOptions};
use crate::sample::{self, SampleClip, SampleSelection};
use wry::{WebViewExtWindows, WebViewBuilderExtWindows, http::{Request, Response}};
use windows_core::{HSTRING, Interface};
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_3, COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_DENY};

const PAGE_URL: &str = "https://enhancer.example/index.html";

enum AppEvent {
    Command(String),
    Loaded(Result<FileData, String>),
    Progress(Value),
    Finished(Result<(FileData, u64, f64), String>),
    Downloaded(Result<PathBuf, String>),
    SampleLoaded(Result<SampleClip, String>),
    SampleRendered(Result<FileData, String>),
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let preview_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tools/previews");
    std::fs::create_dir_all(&preview_root)?;
    let preview_workspace = tempfile::Builder::new().prefix("session-").tempdir_in(preview_root)?;
    let page = preview_workspace.path().join("index.html");
    std::fs::write(&page, include_str!("ui.html"))?;
    std::fs::write(preview_workspace.path().join("enhancements.js"), include_str!("enhancements.js"))?;
    let event_loop = EventLoopBuilder::<AppEvent>::with_user_event().build();
    let window = WindowBuilder::new().with_title("Video Enhancer").with_inner_size(LogicalSize::new(1440.0, 900.0)).with_min_inner_size(LogicalSize::new(900.0, 650.0)).build(&event_loop)?;
    let proxy = event_loop.create_proxy();
    let ipc_proxy = proxy.clone();
    let media_files = Arc::new(Mutex::new(MediaFiles::default()));
    let protocol_files = Arc::clone(&media_files);
    let webview = wry::WebViewBuilder::new()
        .with_https_scheme(true)
        .with_custom_protocol("media".into(), move |_, request| {
            let path = protocol_files.lock().unwrap().paths.get(request.uri().path()).cloned();
            serve_media(&request, path.as_deref()).unwrap_or_else(|_| media_error(500)).map(Into::into)
        })
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
    let mut last_output: Option<PathBuf> = None;
    let mut downloaded_path: Option<PathBuf> = None;
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
                    "load-enhancement-library" => {
                        match load_enhancement_library() {
                            Ok(templates) => emit(json!({"type":"enhancement-library", "templates":templates})),
                            Err(error) => emit(json!({"type":"enhancement-library-error", "message":error.to_string()})),
                        }
                    }
                    "save-enhancement-library" => {
                        let result = if body.len() > MAX_ENHANCEMENT_LIBRARY_BYTES {
                            Err(io::Error::other("Saved enhancements must fit within 1 MiB"))
                        } else { save_enhancement_library(&message["templates"]) };
                        match result {
                            Ok(()) => emit(json!({"type":"enhancement-library-saved"})),
                            Err(error) => emit(json!({"type":"enhancement-library-error", "message":error.to_string()})),
                        }
                    }
                    "open-source" | "open-output" | "show-output" if !busy => {
                        let command = message["command"].as_str().unwrap();
                        let path = if command == "open-source" { &loaded_path } else { &last_output };
                        if let Some(path) = path && let Err(error) = open_video(path, command == "show-output") {
                            emit(json!({"type":"error", "message":error.to_string()}));
                        }
                    }
                    "open-sample" if !busy => {
                        let path = rendered_sample.as_deref().or_else(|| sample_clip.as_ref().map(|clip| clip.video.path.as_path()));
                        if let Some(path) = path && let Err(error) = open_video(path, false) {
                            emit(json!({"type":"sample-error", "message":error.to_string()}));
                        }
                    }
                    "cancel" => { cancelled.store(true, Ordering::Relaxed); }
                    "choose-hdri" if !busy => {
                        if let Some(path) = rfd::FileDialog::new().set_title("Choose a relighting environment").add_filter("HDR environment", &["hdr", "exr", "pfm"]).pick_file() {
                            emit(json!({"type":"hdri-selected", "path":path, "name":path.file_name().unwrap_or_default().to_string_lossy(), "component_id":message["component_id"]}));
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
                    "load" | "load-downloaded" if !busy => {
                        let path = if message["command"] == "load-downloaded" { downloaded_path.clone() } else {
                            rfd::FileDialog::new().set_title("Load video").set_directory(youtube::directory()).add_filter("Video", &["mp4", "mkv", "mov", "avi", "webm", "m4v"]).pick_file()
                        };
                        if let Some(path) = path {
                            busy = true;
                            loaded_path = None;
                            last_output = None;
                            media_files.lock().unwrap().paths.clear();
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
                            }).and_then(|frames| Parser::new(&job.output).get_video_information().map(|video| (video, frames, started.elapsed().as_secs_f64()))).map_err(|error| error.to_string());
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
                        let url = media_files.lock().unwrap().register("source", &source.path);
                        let mut details = video_details(&source);
                        details["url"] = json!(url);
                        emit(json!({"type":"loaded", "source":details}));
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
                        match sample_url(&clip.video.path, preview_workspace.as_ref().unwrap().path()) {
                            Ok(url) => {
                                let mut details = video_details(&clip.video);
                                details["type"] = json!("sample-loaded");
                                details["url"] = json!(url);
                                details["start"] = json!(clip.selection.start);
                                details["actual_duration"] = details["duration"].clone();
                                details["duration"] = json!(clip.selection.duration);
                                emit(details);
                            }
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
                match result.and_then(|video| {
                    let url = sample_url(&video.path, preview_workspace.as_ref().unwrap().path()).map_err(|error| error.to_string())?;
                    let mut details = video_details(&video);
                    details["type"] = json!("sample-rendered");
                    details["url"] = json!(url);
                    rendered_sample = Some(video.path);
                    Ok(details)
                }) {
                    Ok(details) => emit(details),
                    Err(error) => emit(json!({"type":"sample-error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::Downloaded(result)) => {
                busy = false;
                match result {
                    Ok(path) => {
                        emit(json!({"type":"downloaded", "path":path}));
                        downloaded_path = Some(path);
                    }
                    Err(error) => emit(json!({"type":"download-error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            Event::UserEvent(AppEvent::Finished(result)) => {
                busy = false;
                match result {
                    Ok((video, frames, elapsed)) => {
                        let mut details = video_details(&video);
                        details["type"] = json!("finished");
                        details["frames"] = json!(frames);
                        details["elapsed"] = json!(elapsed);
                        details["output"] = json!(video.path);
                        details["url"] = json!(media_files.lock().unwrap().register("output", &video.path));
                        last_output = Some(video.path);
                        emit(details);
                    }
                    Err(error) => emit(json!({"type":"error", "message":error})),
                }
                if close_when_finished { *control_flow = ControlFlow::Exit; }
            }
            _ => {}
        }
    });
}

const MAX_ENHANCEMENT_LIBRARY_BYTES: usize = 1024 * 1024;

fn enhancement_library_path() -> io::Result<PathBuf> {
    let directory = std::env::var_os("LOCALAPPDATA").ok_or_else(|| io::Error::other("Windows local application data folder is unavailable"))?;
    Ok(PathBuf::from(directory).join("VideoEnhancer").join("enhancements.json"))
}

fn validate_enhancement_templates(templates: &Value) -> io::Result<()> {
    let templates = templates.as_array().ok_or_else(|| io::Error::other("Saved enhancements must be a list"))?;
    if templates.len() > 100 { return Err(io::Error::other("Keep at most 100 saved enhancements")); }
    for template in templates {
        if !template.as_object().is_some_and(|fields| fields.len() == 2) {
            return Err(io::Error::other("Each saved enhancement needs a name and settings"));
        }
        let name = template["name"].as_str().ok_or_else(|| io::Error::other("Each saved enhancement needs a name"))?;
        if name.trim().is_empty() || name.chars().count() > 120 {
            return Err(io::Error::other("Enhancement names must contain 1–120 characters"));
        }
        let settings = template["settings"].as_object().ok_or_else(|| io::Error::other("Enhancement settings must be an object"))?;
        if settings.values().any(|value| value.is_array() || value.is_object()) {
            return Err(io::Error::other("Enhancement settings must contain numbers, text, booleans or null"));
        }
    }
    Ok(())
}

fn load_enhancement_library() -> io::Result<Value> {
    let file = match std::fs::File::open(enhancement_library_path()?) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(json!([])),
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.take(MAX_ENHANCEMENT_LIBRARY_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ENHANCEMENT_LIBRARY_BYTES { return Err(io::Error::other("Saved enhancement library exceeds 1 MiB")); }
    let templates: Value = serde_json::from_slice(&bytes)?;
    validate_enhancement_templates(&templates)?;
    Ok(templates)
}

fn save_enhancement_library(templates: &Value) -> io::Result<()> {
    validate_enhancement_templates(templates)?;
    let bytes = serde_json::to_vec_pretty(templates)?;
    if bytes.len() > MAX_ENHANCEMENT_LIBRARY_BYTES { return Err(io::Error::other("Saved enhancements must fit within 1 MiB")); }
    let path = enhancement_library_path()?;
    let directory = path.parent().unwrap();
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    // Persist replaces the old file only after the complete new library has been written.
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

fn sample_url(path: &Path, workspace: &Path) -> io::Result<String> {
    let path = path.canonicalize()?;
    let workspace = workspace.canonicalize()?;
    let relative = path.strip_prefix(workspace).map_err(io::Error::other)?;
    // Sample folders and filenames are generated by the app using URL-safe names.
    Ok(format!("https://enhancer.example/{}", relative.to_string_lossy().replace('\\', "/")))
}

fn video_details(video: &FileData) -> Value {
    let stream = video.metadata["streams"].as_array().and_then(|streams| streams.iter().find(|stream| stream["index"].as_u64() == Some(u64::from(video.video_stream_index))));
    let hdr = stream.and_then(|stream| stream["color_transfer"].as_str()).is_some_and(|transfer| ["smpte2084", "arib-std-b67"].contains(&transfer));
    json!({"name":video.path.file_name().unwrap_or_default().to_string_lossy(), "width":video.width, "height":video.height, "fps":video.fps, "duration":video.duration_seconds, "audio":video.audio_streams.len(), "pixel_format":video.pixel_format, "hdr":hdr})
}

fn open_video(path: &Path, show_location: bool) -> io::Result<()> {
    // The parser canonicalizes paths; Explorer expects ordinary drive/UNC paths.
    let path = path.to_string_lossy();
    let path = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") { format!(r"\\{unc}") } else { path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned() };
    let mut command = std::process::Command::new("explorer.exe");
    if show_location { command.arg("/select,"); }
    command.arg(path).spawn().map(|_| ())
}

#[derive(Default)]
struct MediaFiles {
    paths: HashMap<String, PathBuf>,
    revision: u64,
}

impl MediaFiles {
    fn register(&mut self, slot: &str, path: &Path) -> String {
        let prefix = format!("/{slot}-");
        self.paths.retain(|key, _| !key.starts_with(&prefix));
        self.revision += 1;
        let key = format!("{prefix}{}", self.revision);
        self.paths.insert(key.clone(), path.to_path_buf());
        format!("https://media.localhost{key}")
    }
}

fn media_error(status: u16) -> Response<Vec<u8>> {
    Response::builder().status(status).header("Access-Control-Allow-Origin", "https://enhancer.example").body(Vec::new()).unwrap()
}

fn serve_media(request: &Request<Vec<u8>>, path: Option<&Path>) -> io::Result<Response<Vec<u8>>> {
    let Some(path) = path else { return Ok(media_error(404)); };
    if !matches!(request.method().as_str(), "GET" | "HEAD") { return Ok(media_error(405)); }
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    let mime = match path.extension().and_then(|extension| extension.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "webm" => "video/webm", "mkv" => "video/x-matroska", "mov" => "video/quicktime", "avi" => "video/x-msvideo", _ => "video/mp4",
    };
    let response = Response::builder().header("Content-Type", mime).header("Accept-Ranges", "bytes")
        .header("Access-Control-Allow-Origin", "https://enhancer.example").header("Cache-Control", "no-store");
    if request.method() == "HEAD" { return Ok(response.header("Content-Length", length).body(Vec::new()).unwrap()); }
    // ponytail: one bounded range per request; add multipart only if a player needs it.
    const MAX_BYTES: u64 = 1024 * 1024;
    let range = request.headers().get("Range");
    let (start, end) = if let Some(range) = range {
        match range.to_str().ok().and_then(|range| media_range(range, length)) {
            Some(range) => range,
            None => return Ok(response.status(416).header("Content-Range", format!("bytes */{length}")).body(Vec::new()).unwrap()),
        }
    } else {
        if length > MAX_BYTES { return Ok(media_error(400)); }
        (0, length.saturating_sub(1))
    };
    let count = if length == 0 { 0 } else { (end - start + 1).min(MAX_BYTES) };
    let mut body = vec![0; count as usize];
    file.seek(SeekFrom::Start(start))?;
    file.read_exact(&mut body)?;
    let response = if range.is_some() {
        response.status(206).header("Content-Range", format!("bytes {start}-{}/{length}", start + count - 1))
    } else { response };
    Ok(response.header("Content-Length", count).body(body).unwrap())
}

fn media_range(header: &str, length: u64) -> Option<(u64, u64)> {
    if length == 0 { return None; }
    let (start, end) = header.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() {
        let count = end.parse::<u64>().ok()?;
        return (count > 0).then_some((length.saturating_sub(count), length - 1));
    }
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() { length - 1 } else { end.parse::<u64>().ok()?.min(length - 1) };
    (start <= end && start < length).then_some((start, end))
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
