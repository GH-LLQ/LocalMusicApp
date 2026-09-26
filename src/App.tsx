import {
  useState,
  useEffect,
  useRef,
  useCallback,
  useMemo,
} from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { listen } from "@tauri-apps/api/event";
import { load, type Store } from "@tauri-apps/plugin-store";

import {
  theme,
  Icon,
  IconButton,
  Button,
  TextInput,
  Select,
  RangeSlider,
  type IconName,
} from "./ui";

// ---------- 类型 ----------

type Track = {
  path: string;
  title: string;
  artist: string;
  album: string;
  duration: number;
};

type SortKey = "title" | "artist" | "album" | "duration";
type SortDir = "asc" | "desc";
type PlayMode = "sequence" | "loop" | "single";
type LyricLine = { time: number; text: string };

// ---------- 工具函数 ----------

function formatTime(sec: number) {
  if (!isFinite(sec) || sec < 0) sec = 0;
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function parseLrc(raw: string): LyricLine[] {
  const result: LyricLine[] = [];
  const lineRegex = /\[(\d+):(\d+)(?:[.:](\d+))?\](.*)/;
  const offsetRegex = /\[offset:([+-]?\d+)\]/i;

  let offsetMs = 0;
  const offsetMatch = offsetRegex.exec(raw);
  if (offsetMatch) offsetMs = parseInt(offsetMatch[1], 10);

  for (const line of raw.split(/\r?\n/)) {
    const m = lineRegex.exec(line);
    if (!m) continue;

    const min = parseInt(m[1], 10);
    const sec = parseInt(m[2], 10);
    const msStr = m[3] ?? "0";
    const ms = parseInt(msStr.padEnd(3, "0").slice(0, 3), 10);
    const time = min * 60 + sec + ms / 1000 - offsetMs / 1000;

    const text = m[4].trim();
    if (text) result.push({ time, text });
  }

  return result.sort((a, b) => a.time - b.time);
}

function getAdjacentIndex(
  list: Track[],
  curPath: string,
  dir: 1 | -1,
  allowLoop: boolean
): number | null {
  const idx = list.findIndex((t) => t.path === curPath);
  if (idx === -1) return list.length > 0 ? 0 : null;

  let next = idx + dir;
  if (next < 0) {
    if (allowLoop) next = list.length - 1;
    else return null;
  }
  if (next >= list.length) {
    if (allowLoop) next = 0;
    else return null;
  }
  return next;
}

// ---------- 主组件 ----------

export default function App() {
  const [tracks, setTracks] = useState<Track[]>([]);
  const [loading, setLoading] = useState(false);
  const [current, setCurrent] = useState<Track | null>(null);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [volume, setVolume] = useState(0.8);
  const [seeking, setSeeking] = useState(false);
  const [cover, setCover] = useState<string | null>(null);

  const [query, setQuery] = useState("");
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortDir, setSortDir] = useState<SortDir>("asc");
  const [mode, setMode] = useState<PlayMode>("sequence");

  const [lyrics, setLyrics] = useState<LyricLine[]>([]);
  const [lyricLine, setLyricLine] = useState(-1);
  const lyricsBoxRef = useRef<HTMLDivElement>(null);

  const storeRef = useRef<Store | null>(null);
  const loadedRef = useRef<string | null>(null);  // 记录后端当前加载的 path


  // ---------- 派生：搜索 + 排序 ----------
  const displayTracks = useMemo(() => {
    let result = tracks;

    const q = query.trim().toLowerCase();
    if (q) {
      result = result.filter(
        (t) =>
          t.title.toLowerCase().includes(q) ||
          t.artist.toLowerCase().includes(q) ||
          t.album.toLowerCase().includes(q)
      );
    }

    if (sortKey) {
      result = [...result].sort((a, b) => {
        let cmp = 0;
        if (sortKey === "duration") {
          cmp = a.duration - b.duration;
        } else {
          cmp = (a[sortKey] || "").localeCompare(b[sortKey] || "", "zh");
        }
        return sortDir === "asc" ? cmp : -cmp;
      });
    }

    return result;
  }, [tracks, query, sortKey, sortDir]);

  // ---------- 给事件回调用的最新状态 ----------
  const stateRef = useRef({
    tracks: [] as Track[],
    current: null as Track | null,
    volume: 0.8,
    mode: "sequence" as PlayMode,
  });
  useEffect(() => {
    stateRef.current = { tracks: displayTracks, current, volume, mode };
  }, [displayTracks, current, volume, mode]);

  // ---------- 启动时恢复设置 ----------
  useEffect(() => {
    (async () => {
      try {
        const store = await load("settings.json", { autoSave: true });
        storeRef.current = store;

        const savedTracks = await store.get<Track[]>("tracks");
        const savedVolume = await store.get<number>("volume");
        const savedMode = await store.get<PlayMode>("mode");
        const savedCurrentPath = await store.get<string>("currentPath");

        if (Array.isArray(savedTracks)) setTracks(savedTracks);
        if (typeof savedVolume === "number") setVolume(savedVolume);
        if (savedMode) setMode(savedMode);

        if (savedCurrentPath && Array.isArray(savedTracks)) {
          const found = savedTracks.find((t) => t.path === savedCurrentPath);
          if (found) setCurrent(found);
        }
      } catch (e) {
        console.error("读取设置失败", e);
      }
    })();
  }, []);

  // ---------- 保存 current 路径 ----------
  useEffect(() => {
    if (!storeRef.current) return;
    if (current) {
      storeRef.current.set("currentPath", current.path);
    }
  }, [current]);

  // ---------- 播放指定曲目 ----------
  const playTrack = useCallback(async (track: Track) => {
    try {
      await invoke("play", { path: track.path });
      await invoke("set_volume", { volume: stateRef.current.volume });
      setCurrent(track);
      setPlaying(true);
      setPosition(0);
      loadedRef.current = track.path;   // ← 标记后端已加载
    } catch (e) {
      console.error("播放失败", e);
    }
  }, []);

  // ---------- 监听进度 ----------
  useEffect(() => {
    const unlisten = listen<{ position: number }>("playback-progress", (e) => {
      if (!seeking) setPosition(e.payload.position);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [seeking]);

  // ---------- 监听播放结束 ----------
  useEffect(() => {
    const unlisten = listen("playback-ended", () => {
      const { tracks: list, current: cur, mode: m } = stateRef.current;
      if (!cur || list.length === 0) return;

      if (m === "single") {
        playTrack(cur);
        return;
      }

      const next = getAdjacentIndex(list, cur.path, 1, m === "loop");
      if (next === null) {
        setPlaying(false);
        setPosition(0);
        return;
      }
      playTrack(list[next]);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [playTrack]);

  // ---------- 加载封面 ----------
  useEffect(() => {
    if (!current) {
      setCover(null);
      return;
    }
    let cancelled = false;
    invoke<string | null>("get_cover", { path: current.path })
      .then((data) => {
        if (!cancelled) setCover(data);
      })
      .catch(() => {
        if (!cancelled) setCover(null);
      });
    return () => {
      cancelled = true;
    };
  }, [current?.path]);

  // ---------- 加载歌词 ----------
  useEffect(() => {
    if (!current) {
      setLyrics([]);
      setLyricLine(-1);
      return;
    }
    let cancelled = false;
    invoke<string | null>("get_lyrics", { path: current.path })
      .then((raw) => {
        if (cancelled) return;
        setLyrics(raw ? parseLrc(raw) : []);
      })
      .catch(() => {
        if (!cancelled) setLyrics([]);
      });
    return () => {
      cancelled = true;
    };
  }, [current?.path]);

  // ---------- 根据 position 计算当前歌词行 ----------
  useEffect(() => {
    if (lyrics.length === 0) {
      setLyricLine(-1);
      return;
    }
    let idx = -1;
    for (let i = 0; i < lyrics.length; i++) {
      if (lyrics[i].time <= position) idx = i;
      else break;
    }
    setLyricLine(idx);
  }, [position, lyrics]);

  // ---------- 滚动到当前歌词 ----------
  useEffect(() => {
    if (lyricLine < 0 || !lyricsBoxRef.current) return;
    const el = lyricsBoxRef.current.querySelector<HTMLElement>(
      `[data-line="${lyricLine}"]`
    );
    if (el) {
      el.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  }, [lyricLine]);

  // ---------- 操作函数 ----------

  async function pickFolder() {
    const dir = await open({ directory: true });
    if (typeof dir !== "string") return;

    setLoading(true);
    try {
      const result = await invoke<Track[]>("scan_music_dir", { dir });
      setTracks(result);
      await storeRef.current?.set("musicFolder", dir);
      await storeRef.current?.set("tracks", result);
      await storeRef.current?.set("currentPath", "");
    } catch (e) {
      console.error("扫描失败", e);
      alert("扫描失败：" + e);
    } finally {
      setLoading(false);
    }
  }

  async function togglePlay() {
    if (!current) return;

      // 后端还没加载这首歌（比如刚启动、从 store 恢复的），先走 play
    if (loadedRef.current !== current.path) {
      await playTrack(current);
      return;
    }


    try {
      if (playing) {
        await invoke("pause");
        setPlaying(false);
      } else {
        await invoke("resume");
        setPlaying(true);
      }
    } catch (e) {
      console.error(e);
    }
  }

  function handleNext() {
    if (!current) return;
    const next = getAdjacentIndex(displayTracks, current.path, 1, mode === "loop");
    if (next !== null) playTrack(displayTracks[next]);
  }

  function handlePrev() {
    if (!current) return;
    const prev = getAdjacentIndex(displayTracks, current.path, -1, mode === "loop");
    if (prev !== null) playTrack(displayTracks[prev]);
  }

  function handleRowButtonClick(e: React.MouseEvent, track: Track) {
    e.stopPropagation();   // 阻止冒泡，避免触发行的 onDoubleClick
    if (current?.path === track.path) {
      togglePlay();        // 当前曲目 → 暂停/继续
    } else {
      playTrack(track);    // 其他曲目 → 播放
    }
  }

  function cycleMode() {
    setMode((m) => {
      const next =
        m === "sequence" ? "loop" : m === "loop" ? "single" : "sequence";
      storeRef.current?.set("mode", next);
      return next;
    });
  }

  async function handleSeekChange(v: number) {
    setPosition(v);
    await invoke("seek", { seconds: v });
  }

  async function handleVolumeChange(v: number) {
    setVolume(v);
    await invoke("set_volume", { volume: v });
    storeRef.current?.set("volume", v);
  }

  function handleSortChange(value: string) {
    if (value === "default") {
      setSortKey(null);
      return;
    }
    const [key, dir] = value.split(":") as [SortKey, SortDir];
    setSortKey(key);
    setSortDir(dir);
  }

  // ---------- 渲染 ----------

  const duration = current?.duration ?? 0;
  const sortValue = sortKey ? `${sortKey}:${sortDir}` : "default";

  const modeIcon: IconName =
    mode === "sequence" ? "sequence" : mode === "loop" ? "loop" : "single";
  const modeTitle =
    mode === "sequence"
      ? "顺序播放（播完最后一首停止）"
      : mode === "loop"
      ? "列表循环"
      : "单曲循环";

  const isMac = navigator.userAgent.includes("Mac");

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        fontFamily: "system-ui",
        color: theme.colors.text,
        background: theme.colors.bg,
        overflow: "hidden",
      }}
    >
      {/* 顶部工具栏（可拖动窗口） */}
      <div
        style={{
          padding: "10px 16px",
          paddingLeft: isMac ? 80 : 16,
          borderBottom: `1px solid ${theme.colors.border}`,
          background: theme.colors.panel,
          display: "flex",
          alignItems: "center",
          gap: 10,
          flexWrap: "wrap",
          WebkitAppRegion: "drag",
        } as React.CSSProperties}
      >
        <div
          style={{
            WebkitAppRegion: "no-drag",
            display: "flex",
            alignItems: "center",
            gap: 10,
            flex: 1,
          } as React.CSSProperties}
        >
          <Button onClick={pickFolder} disabled={loading}>
            {loading ? "扫描中…" : "选择音乐文件夹"}
          </Button>

          <TextInput
            value={query}
            onChange={setQuery}
            placeholder="搜索标题 / 艺术家 / 专辑…"
            style={{ flex: 1, minWidth: 180 }}
          />

          <Select
            value={sortValue}
            onChange={handleSortChange}
            options={[
              { value: "default", label: "默认顺序" },
              { value: "title:asc", label: "标题 A→Z" },
              { value: "title:desc", label: "标题 Z→A" },
              { value: "artist:asc", label: "艺术家 A→Z" },
              { value: "artist:desc", label: "艺术家 Z→A" },
              { value: "album:asc", label: "专辑 A→Z" },
              { value: "album:desc", label: "专辑 Z→A" },
              { value: "duration:asc", label: "时长 短→长" },
              { value: "duration:desc", label: "时长 长→短" },
            ]}
          />

          <span
            style={{
              color: theme.colors.textDim,
              fontSize: 13,
              whiteSpace: "nowrap",
            }}
          >
            {query
              ? `${displayTracks.length} / ${tracks.length} 首`
              : `共 ${tracks.length} 首`}
          </span>
        </div>
      </div>

      {/* 中间：列表 + 歌词面板 */}
      <div style={{ flex: 1, display: "flex", overflow: "hidden" }}>
        {/* 歌曲列表 */}
        <ul
          style={{
            listStyle: "none",
            margin: 0,
            padding: 0,
            flex: 2,
            overflowY: "auto",
          }}
        >
         {displayTracks.map((t) => {
            const isCurrent = current?.path === t.path;
            const isPlayingThis = isCurrent && playing;

            return (
              <li
                key={t.path}
                className={`track-row${isCurrent ? " is-current" : ""}`}
                onDoubleClick={() => playTrack(t)}
              >
                {/* 左侧播放按钮（占固定宽度，hover 时出现） */}
                <button
                  className="track-play-btn"
                  onClick={(e) => handleRowButtonClick(e, t)}
                  title={isPlayingThis ? "暂停" : "播放"}
                >
                  <Icon name={isPlayingThis ? "pause" : "play"} size={16} />
                </button>

                {/* 歌曲信息 */}
                <span
                  style={{
                    flex: 1,
                    padding: "10px 0",
                    overflow: "hidden",
                    textOverflow: "ellipsis",
                    whiteSpace: "nowrap",
                  }}
                >
                  <strong>{t.title}</strong>
                  {t.artist && (
                    <span style={{ color: theme.colors.textDim }}> — {t.artist}</span>
                  )}
                  {t.album && (
                    <span style={{ color: theme.colors.textMuted, fontSize: 12 }}>
                      {" "}
                      · {t.album}
                    </span>
                  )}
                </span>

                {/* 时长 */}
                <span
                  style={{
                    color: theme.colors.textDim,
                    flexShrink: 0,
                    marginLeft: 12,
                    paddingRight: 16,
                  }}
                >
                  {formatTime(t.duration)}
                </span>
              </li>
            );
          })}
        </ul>

        {/* 歌词面板 */}
        <div
          ref={lyricsBoxRef}
          style={{
            flex: 1,
            borderLeft: `1px solid ${theme.colors.border}`,
            overflowY: "auto",
            padding: "40px 20px",
            background: theme.colors.panel,
          }}
        >
          {lyrics.length === 0 ? (
            <div
              style={{
                color: theme.colors.textMuted,
                textAlign: "center",
                marginTop: 40,
                fontSize: 14,
              }}
            >
              {current ? "暂无歌词" : "未播放"}
            </div>
          ) : (
            lyrics.map((line, i) => (
              <div
                key={i}
                data-line={i}
                onClick={() => handleSeekChange(line.time)}
                style={{
                  padding: "6px 0",
                  color:
                    i === lyricLine
                      ? theme.colors.accent
                      : i < lyricLine
                      ? theme.colors.textMuted
                      : theme.colors.textDim,
                  fontSize: i === lyricLine ? 16 : 14,
                  fontWeight: i === lyricLine ? 600 : 400,
                  textAlign: "center",
                  cursor: "pointer",
                  transition: "color 0.2s, font-size 0.2s",
                  lineHeight: 1.5,
                }}
              >
                {line.text}
              </div>
            ))
          )}
        </div>
      </div>

      {/* 底部播放栏 */}
      <div
        style={{
          padding: "12px 16px",
          borderTop: `1px solid ${theme.colors.border}`,
          background: theme.colors.panel,
          display: "flex",
          flexDirection: "column",
          gap: 8,
        }}
      >
        {/* 进度条行 */}
        <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
          <span
            style={{
              color: theme.colors.textDim,
              width: 40,
              textAlign: "right",
              fontSize: 12,
            }}
          >
            {formatTime(position)}
          </span>
          <RangeSlider
            value={Math.min(position, duration)}
            max={duration || 0}
            step={0.1}
            onChange={setPosition}
            onMouseDown={() => setSeeking(true)}
            onMouseUp={(v) => {
              setSeeking(false);
              handleSeekChange(v);
            }}
            disabled={!current}
            style={{ flex: 1 }}
          />
          <span
            style={{ color: theme.colors.textDim, width: 40, fontSize: 12 }}
          >
            {formatTime(duration)}
          </span>
        </div>

        {/* 控制行 */}
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          {/* 封面 */}
          <div
            style={{
              width: 56,
              height: 56,
              borderRadius: theme.radius.md,
              overflow: "hidden",
              background: theme.colors.elevated,
              flexShrink: 0,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
              fontSize: 24,
              color: theme.colors.textMuted,
              marginRight: 4,
            }}
          >
            {cover ? (
              <img
                src={cover}
                alt="cover"
                style={{ width: "100%", height: "100%", objectFit: "cover" }}
              />
            ) : (
              "🎵"
            )}
          </div>

          <IconButton onClick={handlePrev} disabled={!current} title="上一首">
            <Icon name="prev" />
          </IconButton>

          <IconButton
            onClick={togglePlay}
            disabled={!current}
            title={playing ? "暂停" : "播放"}
          >
            <Icon name={playing ? "pause" : "play"} size={22} />
          </IconButton>

          <IconButton onClick={handleNext} disabled={!current} title="下一首">
            <Icon name="next" />
          </IconButton>

          <IconButton
            onClick={cycleMode}
            title={modeTitle}
            active={mode !== "sequence"}
          >
            <Icon name={modeIcon} />
          </IconButton>

          <span
            style={{
              flex: 1,
              color: theme.colors.textDim,
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
              marginLeft: 8,
            }}
          >
            {current
              ? `${current.title}${current.artist ? " — " + current.artist : ""}`
              : "未播放"}
          </span>

          <span
            style={{
              color: theme.colors.textDim,
              display: "flex",
              alignItems: "center",
            }}
          >
            <Icon
              name={
                volume === 0 ? "volumeMute" : volume < 0.5 ? "volumeLow" : "volume"
              }
              size={18}
            />
          </span>
          <RangeSlider
            value={volume}
            max={1}
            onChange={handleVolumeChange}
            style={{ width: 100 }}
          />
        </div>
      </div>
    </div>
  );
}