use base64::{engine::general_purpose, Engine as _};
use cpal::traits::{DeviceTrait, HostTrait};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::ItemKey;
use rodio::{OutputStream, OutputStreamHandle, Sink};
use serde::Serialize;
use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;
use walkdir::WalkDir;

mod decoder;
use decoder::SymphoniaSource;

#[derive(Serialize)]
struct Track {
    path: String,
    title: String,
    artist: String,
    album: String,
    duration: u64,
}

// ---- 音频播放 ----

enum AudioCmd {
    Play(String),
    Pause,
    Resume,
    Stop,
    Seek(f64),      // 秒
    SetVolume(f32), // 0.0 ~ 1.0
}

#[derive(Debug)]
enum MediaCmd {
    Update {
        playing: bool,
        title: String,
        artist: String,
        album: String,
        duration: Option<Duration>,
        position: Option<Duration>,
    },
}

#[tauri::command]
fn update_media_state(
    state: tauri::State<std::sync::Mutex<Sender<MediaCmd>>>,
    playing: bool,
    title: String,
    artist: String,
    album: String,
    duration_secs: u64,
    position_secs: f64,
) -> Result<(), String> {
    let _ = state.lock().unwrap().send(MediaCmd::Update {
        playing,
        title,
        artist,
        album,
        duration: Some(Duration::from_secs(duration_secs)),
        position: Some(Duration::from_secs_f64(position_secs)),
    });
    Ok(())
}

#[derive(Serialize, Clone)]
struct PlaybackProgress {
    position: f64,
}

struct AudioPlayer {
    tx: Sender<AudioCmd>,
}

#[tauri::command]
fn play(state: tauri::State<AudioPlayer>, path: String) -> Result<(), String> {
    state
        .tx
        .send(AudioCmd::Play(path))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn pause(state: tauri::State<AudioPlayer>) -> Result<(), String> {
    state.tx.send(AudioCmd::Pause).map_err(|e| e.to_string())
}

#[tauri::command]
fn resume(state: tauri::State<AudioPlayer>) -> Result<(), String> {
    state.tx.send(AudioCmd::Resume).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop(state: tauri::State<AudioPlayer>) -> Result<(), String> {
    state.tx.send(AudioCmd::Stop).map_err(|e| e.to_string())
}

#[tauri::command]
fn seek(state: tauri::State<AudioPlayer>, seconds: f64) -> Result<(), String> {
    state
        .tx
        .send(AudioCmd::Seek(seconds))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_volume(state: tauri::State<AudioPlayer>, volume: f32) -> Result<(), String> {
    state
        .tx
        .send(AudioCmd::SetVolume(volume))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_cover(path: String) -> Result<Option<String>, String> {
    let tagged = match lofty::read_from_path(&path) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };

    for tag in tagged.tags() {
        if let Some(pic) = tag.pictures().first() {
            let mime = pic.mime_type().map(|m| m.as_str()).unwrap_or("image/jpeg");
            let b64 = general_purpose::STANDARD.encode(pic.data());
            return Ok(Some(format!("data:{};base64,{}", mime, b64)));
        }
    }

    Ok(None)
}

#[tauri::command]
fn get_lyrics(path: String) -> Result<Option<String>, String> {
    let p = std::path::Path::new(&path);
    let lrc_path = p.with_extension("lrc");

    if lrc_path.exists() {
        std::fs::read_to_string(&lrc_path)
            .map(Some)
            .map_err(|e| e.to_string())
    } else {
        Ok(None)
    }
}

#[tauri::command]
fn scan_music_dir(dir: String) -> Result<Vec<Track>, String> {
    let mut tracks = Vec::new();

    for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if !matches!(ext.as_str(), "mp3" | "flac" | "m4a" | "wav" | "ogg" | "aac") {
            continue;
        }

        let (title, artist, album, duration) = match lofty::read_from_path(path) {
            Ok(tagged) => {
                let duration = tagged.properties().duration().as_secs();

                let mut title = String::new();
                let mut artist = String::new();
                let mut album = String::new();

                for tag in tagged.tags() {
                    for item in tag.items() {
                        match item.key() {
                            ItemKey::TrackTitle => {
                                if let Some(t) = item.value().text() {
                                    title = t.to_string();
                                }
                            }
                            ItemKey::TrackArtist => {
                                if let Some(t) = item.value().text() {
                                    artist = t.to_string();
                                }
                            }
                            ItemKey::AlbumTitle => {
                                if let Some(t) = item.value().text() {
                                    album = t.to_string();
                                }
                            }
                            _ => {}
                        }
                    }
                }

                (title, artist, album, duration)
            }
            Err(_) => (String::new(), String::new(), String::new(), 0),
        };

        let title = if title.is_empty() {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            title
        };

        tracks.push(Track {
            path: path.to_string_lossy().to_string(),
            title,
            artist,
            album,
            duration,
        });
    }
    eprintln!("扫描到 {} 首歌", tracks.len());
    Ok(tracks)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .setup(|app| {
            // ====== 初始化蓝牙/系统媒体控制 ======
            #[cfg(target_os = "windows")]
            let hwnd_val = {
                let window = app.get_webview_window("main").unwrap();
                window.hwnd().unwrap().0 as usize
            };
            #[cfg(not(target_os = "windows"))]
            let _hwnd_val = 0usize;

            #[cfg(target_os = "windows")]
            let hwnd = if hwnd_val != 0 {
                Some(hwnd_val as *mut std::ffi::c_void)
            } else {
                None
            };
            #[cfg(not(target_os = "windows"))]
            let hwnd = None;

            let config = PlatformConfig {
                dbus_name: "muse",
                display_name: "Muse",
                hwnd,
            };

            let (media_tx, media_rx) = mpsc::channel::<MediaCmd>();
            app.manage(std::sync::Mutex::new(media_tx));

            // 【关键】在主线程初始化 MediaControls
            match MediaControls::new(config) {
                Ok(mut controls) => {
                    println!("系统媒体控制初始化成功");
                    let app_handle_media = app.handle().clone();

                    if let Err(e) = controls.attach(move |event| {
                        eprintln!("[Souvlaki] 收到系统媒体事件: {:?}", event);
                        match event {
                            MediaControlEvent::Toggle | MediaControlEvent::Play => {
                                let _ = app_handle_media.emit("hw-media-play", ());
                            }
                            MediaControlEvent::Pause => {
                                let _ = app_handle_media.emit("hw-media-pause", ());
                            }
                            MediaControlEvent::Next => {
                                let _ = app_handle_media.emit("hw-media-next", ());
                            }
                            MediaControlEvent::Previous => {
                                let _ = app_handle_media.emit("hw-media-prev", ());
                            }
                            _ => {}
                        }
                    }) {
                        eprintln!("媒体按键绑定失败: {:?}", e);
                    }

                    // 移交给后台线程处理状态更新
                    std::thread::spawn(move || {
                        while let Ok(cmd) = media_rx.recv() {
                            match cmd {
                                MediaCmd::Update {
                                    playing,
                                    title,
                                    artist,
                                    album,
                                    duration,
                                    position,
                                } => {
                                    let metadata = souvlaki::MediaMetadata {
                                        title: Some(&title),
                                        artist: Some(&artist),
                                        album: Some(&album),
                                        duration,
                                        cover_url: None,
                                    };
                                    let _ = controls.set_metadata(metadata);

                                    let playback = if playing {
                                        souvlaki::MediaPlayback::Playing {
                                            progress: position.map(souvlaki::MediaPosition),
                                        }
                                    } else {
                                        souvlaki::MediaPlayback::Paused {
                                            progress: position.map(souvlaki::MediaPosition),
                                        }
                                    };
                                    let _ = controls.set_playback(playback);
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("系统媒体控制初始化失败: {:?}", e);
                }
            }

            // ====== 音频播放线程 ======
            let app_handle = app.handle().clone();
            let (tx, rx) = mpsc::channel::<AudioCmd>();

            thread::spawn(move || {
                let host = cpal::default_host();

                let create_output = || -> Option<(OutputStream, OutputStreamHandle)> {
                    match OutputStream::try_default() {
                        Ok(pair) => Some(pair),
                        Err(e) => {
                            eprintln!("音频设备初始化失败: {}", e);
                            None
                        }
                    }
                };

                let device_name = |host: &cpal::Host| -> Option<String> {
                    host.default_output_device().and_then(|d| d.name().ok())
                };

                let mut output = create_output();
                let mut current_device = device_name(&host);

                let mut sink: Option<Sink> = None;
                let mut current_path: Option<String> = None;
                let mut current_volume: f32 = 1.0;
                let mut base_offset: f64 = 0.0;
                let mut ended_reported = false;

                loop {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(cmd) => match cmd {
                            AudioCmd::Play(path) => {
                                if let Some(s) = &sink {
                                    s.stop();
                                }
                                sink = None;

                                if output.is_none() {
                                    output = create_output();
                                    current_device = device_name(&host);
                                }

                                if let Some((_, handle)) = &output {
                                    match SymphoniaSource::open(&path) {
                                        Ok(source) => match Sink::try_new(handle) {
                                            Ok(new_sink) => {
                                                new_sink.set_volume(current_volume);
                                                new_sink.append(source);
                                                new_sink.play();
                                                sink = Some(new_sink);
                                                current_path = Some(path);
                                                base_offset = 0.0;
                                                ended_reported = false;
                                            }
                                            Err(e) => eprintln!("创建 sink 失败: {}", e),
                                        },
                                        Err(e) => eprintln!("解码失败 {}: {}", path, e),
                                    }
                                }
                            }
                            AudioCmd::Pause => {
                                if let Some(s) = &sink {
                                    s.pause();
                                }
                            }
                            AudioCmd::Resume => {
                                let is_empty = sink.as_ref().map_or(true, |s| s.empty());
                                if is_empty {
                                    if let Some(path) = current_path.clone() {
                                        if let Some(s) = &sink {
                                            s.stop();
                                        }
                                        sink = None;
                                        if output.is_none() {
                                            output = create_output();
                                            current_device = device_name(&host);
                                        }
                                        if let Some((_, handle)) = &output {
                                            if let Ok(source) = SymphoniaSource::open(&path) {
                                                if let Ok(new_sink) = Sink::try_new(handle) {
                                                    new_sink.set_volume(current_volume);
                                                    new_sink.append(source);
                                                    new_sink.play();
                                                    sink = Some(new_sink);
                                                    base_offset = 0.0;
                                                    ended_reported = false;
                                                }
                                            }
                                        }
                                    }
                                } else if let Some(s) = &sink {
                                    s.play();
                                }
                            }
                            AudioCmd::Stop => {
                                if let Some(s) = &sink {
                                    s.stop();
                                }
                                sink = None;
                            }
                            AudioCmd::Seek(sec) => {
                                if let Some(path) = current_path.clone() {
                                    let was_playing =
                                        sink.as_ref().map_or(false, |s| !s.is_paused());
                                    if let Some(s) = &sink {
                                        s.stop();
                                    }
                                    sink = None;

                                    if output.is_none() {
                                        output = create_output();
                                        current_device = device_name(&host);
                                    }

                                    if let Some((_, handle)) = &output {
                                        if let Ok(mut source) = SymphoniaSource::open(&path) {
                                            use rodio::Source;
                                            if let Err(e) =
                                                source.try_seek(Duration::from_secs_f64(sec))
                                            {
                                                eprintln!("seek 失败: {:?}", e);
                                            }
                                            if let Ok(new_sink) = Sink::try_new(handle) {
                                                new_sink.set_volume(current_volume);
                                                new_sink.append(source);
                                                if was_playing {
                                                    new_sink.play();
                                                } else {
                                                    new_sink.pause();
                                                }
                                                sink = Some(new_sink);
                                                base_offset = sec;
                                                ended_reported = false;
                                            }
                                        }
                                    }
                                }
                            }
                            AudioCmd::SetVolume(v) => {
                                current_volume = v;
                                if let Some(s) = &sink {
                                    s.set_volume(v);
                                }
                            }
                        },
                        Err(RecvTimeoutError::Timeout) => {
                            // 检测设备变化
                            let new_device = device_name(&host);
                            if new_device != current_device {
                                eprintln!("音频设备变化: {:?} -> {:?}", current_device, new_device);
                                

                                let pos = base_offset
                                    + sink
                                        .as_ref()
                                        .map(|s| s.get_pos().as_secs_f64())
                                        .unwrap_or(0.0);
                                let playing = sink.as_ref().map_or(false, |s| !s.is_paused());
                                let path = current_path.clone();

                                if let Some(s) = &sink {
                                    s.stop();
                                }
                                sink = None;
                                

                                output = create_output();
                                current_device = device_name(&host);

                                if let (Some((_, handle)), Some(path)) = (&output, path) {
                                    match SymphoniaSource::open(&path) {
                                        Ok(mut source) => {
                                            use rodio::Source;
                                            let _ = source.try_seek(Duration::from_secs_f64(pos));
                                            match Sink::try_new(handle) {
                                                Ok(new_sink) => {
                                                    new_sink.set_volume(current_volume);
                                                    new_sink.append(source);
                                                    if playing {
                                                        new_sink.play();
                                                    } else {
                                                        new_sink.pause();
                                                    }
                                                    sink = Some(new_sink);
                                                    base_offset = pos;
                                                    ended_reported = false;
                                                }
                                                Err(e) => eprintln!("重建 sink 失败: {}", e),
                                            }
                                        }
                                        Err(e) => eprintln!("重开文件失败 {}: {}", path, e),
                                    }
                                }
                            }

                            // 进度上报
                            if let Some(s) = &sink {
                                if s.empty() {
                                    if !ended_reported {
                                        ended_reported = true;
                                        let _ = app_handle.emit("playback-ended", ());
                                    }
                                } else {
                                    ended_reported = false;
                                    let position = base_offset + s.get_pos().as_secs_f64();
                                    let _ = app_handle
                                        .emit("playback-progress", PlaybackProgress { position });
                                }
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
            });

            app.manage(AudioPlayer { tx });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_music_dir,
            play,
            pause,
            resume,
            stop,
            seek,
            set_volume,
            get_cover,
            get_lyrics,
            update_media_state
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}