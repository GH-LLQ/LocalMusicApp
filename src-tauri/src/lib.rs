use serde::Serialize;
use walkdir::WalkDir;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::ItemKey;
use std::sync::mpsc::{self, Sender};
use std::thread;
use rodio::{OutputStream, Sink};
use tauri::Manager;
mod decoder;
use decoder::SymphoniaSource;
use std::time::Duration;
use std::sync::mpsc::RecvTimeoutError;
use tauri::Emitter;
use base64::{engine::general_purpose, Engine as _};

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
    Seek(f64),       // 秒
    SetVolume(f32),  // 0.0 ~ 1.0
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
    state.tx.send(AudioCmd::Play(path)).map_err(|e| e.to_string())
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
    state.tx.send(AudioCmd::Seek(seconds)).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_volume(state: tauri::State<AudioPlayer>, volume: f32) -> Result<(), String> {
    state.tx.send(AudioCmd::SetVolume(volume)).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_cover(path: String) -> Result<Option<String>, String> {
    let tagged = match lofty::read_from_path(&path) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };

    for tag in tagged.tags() {
        if let Some(pic) = tag.pictures().first() {
            let mime = pic
                .mime_type()
                .map(|m| m.as_str())
                .unwrap_or("image/jpeg");
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
            let app_handle = app.handle().clone();
            let (tx, rx) = mpsc::channel::<AudioCmd>();
        
            thread::spawn(move || {
                let (_stream, stream_handle) = match OutputStream::try_default() {
                    Ok(pair) => pair,
                    Err(e) => {
                        eprintln!("音频设备初始化失败: {}", e);
                        return;
                    }
                };
        
                let mut sink: Option<Sink> = None;
                let mut current_path: Option<String> = None;
                let mut current_volume: f32 = 1.0;
                let mut base_offset: f64 = 0.0;
                let mut ended_reported = false;
        
                loop {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(cmd) => match cmd {
                            AudioCmd::Play(path) => {
                                if let Some(s) = &sink {
                                    s.stop();
                                }
                                match SymphoniaSource::open(&path) {
                                    Ok(source) => match Sink::try_new(&stream_handle) {
                                        Ok(new_sink) => {
                                            new_sink.set_volume(current_volume);
                                            new_sink.append(source);
                                            new_sink.play();
                                            sink = Some(new_sink);
                                            current_path = Some(path);
                                            base_offset = 0.0;      // ← 新歌，位置归零
                                        }
                                        Err(e) => eprintln!("创建 sink 失败: {}", e),
                                    },
                                    Err(e) => eprintln!("解码失败 {}: {}", path, e),
                                }
                                ended_reported = false;
                            }
                            AudioCmd::Pause => {
                                if let Some(s) = &sink {
                                    s.pause();
                                }
                            }
                            AudioCmd::Resume => {
                                let is_empty = sink.as_ref().map_or(true, |s| s.empty());
                            
                                if is_empty {
                                    // 已经播完（或没有 sink），重新从当前歌开头播
                                    if let Some(path) = current_path.clone() {
                                        if let Some(old) = sink.take() {
                                            old.stop();
                                        }
                                        match SymphoniaSource::open(&path) {
                                            Ok(source) => match Sink::try_new(&stream_handle) {
                                                Ok(new_sink) => {
                                                    new_sink.set_volume(current_volume);
                                                    new_sink.append(source);
                                                    new_sink.play();
                                                    sink = Some(new_sink);
                                                    base_offset = 0.0;
                                                    ended_reported = false;
                                                }
                                                Err(e) => eprintln!("创建 sink 失败: {}", e),
                                            },
                                            Err(e) => eprintln!("重开文件失败 {}: {}", path, e),
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
                                    // 记住当前是否正在播放
                                    let was_playing = sink.as_ref().map_or(false, |s| !s.is_paused());
                            
                                    // 停掉旧 sink
                                    if let Some(s) = &sink {
                                        s.stop();
                                    }
                            
                                    // 重新打开同一个文件
                                    match SymphoniaSource::open(&path) {
                                        Ok(mut source) => {
                                            use rodio::Source;
                                            if let Err(e) = source.try_seek(Duration::from_secs_f64(sec)) {
                                                eprintln!("seek 失败: {:?}", e);
                                            }
                            
                                            match Sink::try_new(&stream_handle) {
                                                Ok(new_sink) => {
                                                    new_sink.set_volume(current_volume);
                                                    new_sink.append(source);
                                                    if was_playing {
                                                        new_sink.play();
                                                    } else {
                                                        new_sink.pause();
                                                    }
                                                    sink = Some(new_sink);
                                                    base_offset = sec;      // ← 关键：记下目标位置
                                                }
                                                Err(e) => eprintln!("创建 sink 失败: {}", e),
                                            }
                                        }
                                        Err(e) => eprintln!("重开文件失败 {}: {}", path, e),
                                    }
                                }
                                ended_reported = false;
                            }
                            AudioCmd::SetVolume(v) => {
                                current_volume = v;
                                if let Some(s) = &sink {
                                    s.set_volume(v);
                                }
                            }
                        },
                        Err(RecvTimeoutError::Timeout) => {
                            if let Some(s) = &sink {
                                if s.empty() {
                                    // 播放结束
                                    if !ended_reported {
                                        ended_reported = true;
                                        let _ = app_handle.emit("playback-ended", ());
                                    }
                                } else {
                                    ended_reported = false;
                                    let position = base_offset + s.get_pos().as_secs_f64();
                                    let _ = app_handle.emit(
                                        "playback-progress",
                                        PlaybackProgress { position },
                                    );
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
            get_lyrics
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}