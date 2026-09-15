// Development harness for the player: renders VideoSurfaces against the mock backend.
// `npm run dev`, then open /player-harness.html. Not part of the app.

import { StrictMode, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode, type Ref } from "react";
import { createRoot } from "react-dom/client";
import type { StreamQuality, StreamRequest } from "../../ipc/api";
import { mockNetwork, mockStreamCounters } from "../../ipc/mock/stream";
import type { PlayerState, PlayerStats, VideoSurfaceHandle } from "../types";
import { VideoSurface } from "../VideoSurface";

type Mode = "live" | "playback";

const CAMERAS = [
  { id: "mock-h264", label: "H.264 720p15 (loops)" },
  { id: "mock-h265", label: "H.265 360p15" },
  { id: "mock-short", label: "H.264, ends after 10 s" },
  { id: "mock-unsupported", label: "Unsupported codec (simulated)" },
];
const SPEEDS = [0.5, 1, 2, 4, 8, 16];
const GRID_CAMERAS = ["mock-grid-2", "mock-grid-3", "mock-grid-4"];

interface TileReport {
  state: PlayerState;
  stats?: PlayerStats;
  timeMs?: number;
  /** Recent state changes, for scripted checks: [performance.now(), kind]. */
  history: [number, string][];
}

/** Exposed for scripted checks: `window.__harness.tiles[0].stats` etc. */
const reports: TileReport[] = [];
(window as unknown as { __harness: object }).__harness = { tiles: reports, network: mockNetwork, streams: mockStreamCounters };

function isoMinus(ms: number): string {
  return new Date(Date.now() - ms).toISOString();
}

function Harness() {
  const [mode, setMode] = useState<Mode>("live");
  const [camera, setCamera] = useState(CAMERAS[0].id);
  const [quality, setQuality] = useState<StreamQuality>("hd");
  const [speed, setSpeed] = useState(1);
  const [start, setStart] = useState(() => isoMinus(3600_000));
  const [startInput, setStartInput] = useState(start);
  const [grid, setGrid] = useState(false);
  const [muted, setMuted] = useState(true);
  const [fit, setFit] = useState<"contain" | "cover">("contain");
  const [mounted, setMounted] = useState(true);
  const [spacer, setSpacer] = useState(false);
  const [jitter, setJitter] = useState(0);
  const [corrupt, setCorrupt] = useState(0);
  const [snapshotUrl, setSnapshotUrl] = useState<string>();
  const [snapshotInfo, setSnapshotInfo] = useState("");
  const [, setTick] = useState(0);
  const mainTile = useRef<VideoSurfaceHandle>(null);
  const currentTime = useRef<number | undefined>(undefined);

  useEffect(() => {
    mockNetwork.jitterMs = jitter;
    mockNetwork.corruptRate = corrupt;
  }, [jitter, corrupt]);

  // Re-render twice a second for the readouts.
  useEffect(() => {
    const timer = setInterval(() => setTick((t) => t + 1), 500);
    return () => clearInterval(timer);
  }, []);

  const sourceFor = useMemo(
    () =>
      (cameraId: string): StreamRequest =>
        mode === "live" ? { kind: "live", cameraId, quality } : { kind: "playback", cameraId, start, speed },
    [mode, quality, start, speed],
  );

  /** Restarts playback at `ms` (epoch), keeping speed. */
  const seekTo = (ms: number) => {
    const iso = new Date(ms).toISOString();
    setStart(iso);
    setStartInput(iso);
  };
  const seekBy = (deltaMs: number) => seekTo((currentTime.current ?? Date.parse(start)) + deltaMs);
  const changeSpeed = (value: number) => {
    // Continue from the current position at the new speed.
    if (currentTime.current !== undefined) seekTo(currentTime.current);
    setSpeed(value);
  };

  const takeSnapshot = async () => {
    try {
      const blob = await mainTile.current!.snapshot();
      const bitmap = await createImageBitmap(blob);
      setSnapshotInfo(`${blob.type} ${bitmap.width}×${bitmap.height}, ${(blob.size / 1024).toFixed(0)} KiB`);
      bitmap.close();
      setSnapshotUrl((old) => {
        if (old) URL.revokeObjectURL(old);
        return URL.createObjectURL(blob);
      });
    } catch (error) {
      setSnapshotInfo(`snapshot failed: ${error instanceof Error ? error.message : error}`);
    }
  };

  const cameras = grid ? [camera, ...GRID_CAMERAS] : [camera];
  return (
    <div style={page}>
      <header style={toolbar}>
        <strong>Player harness</strong>
        <Group label="Mode">
          {(["live", "playback"] as const).map((m) => (
            <label key={m}>
              <input type="radio" checked={mode === m} onChange={() => setMode(m)} /> {m}
            </label>
          ))}
        </Group>
        <Group label="Camera">
          <select value={camera} onChange={(e) => setCamera(e.target.value)}>
            {CAMERAS.map((c) => (
              <option key={c.id} value={c.id}>
                {c.label}
              </option>
            ))}
          </select>
        </Group>
        {mode === "live" ? (
          <Group label="Quality">
            <select value={quality} onChange={(e) => setQuality(e.target.value as StreamQuality)}>
              <option value="hd">hd</option>
              <option value="sd">sd</option>
            </select>
          </Group>
        ) : (
          <>
            <Group label="Speed">
              <select value={speed} onChange={(e) => changeSpeed(Number(e.target.value))} data-testid="speed">
                {SPEEDS.map((s) => (
                  <option key={s} value={s}>
                    {s}×
                  </option>
                ))}
              </select>
            </Group>
            <Group label="Seek">
              <button onClick={() => seekBy(-30_000)}>−30 s</button>
              <button onClick={() => seekBy(30_000)}>+30 s</button>
              <input
                style={{ width: 210 }}
                value={startInput}
                onChange={(e) => setStartInput(e.target.value)}
                data-testid="start"
              />
              <button onClick={() => !Number.isNaN(Date.parse(startInput)) && seekTo(Date.parse(startInput))}>Go</button>
            </Group>
          </>
        )}
        <Group label="Layout">
          <label>
            <input type="checkbox" checked={grid} onChange={(e) => setGrid(e.target.checked)} /> 2×2
          </label>
          <select value={fit} onChange={(e) => setFit(e.target.value as "contain" | "cover")}>
            <option value="contain">contain</option>
            <option value="cover">cover</option>
          </select>
        </Group>
        <Group label="Audio">
          <button onClick={() => setMuted((m) => !m)} data-testid="mute">
            {muted ? "Unmute" : "Mute"}
          </button>
        </Group>
        <Group label="Network">
          <select value={jitter} onChange={(e) => setJitter(Number(e.target.value))} title="jitter">
            {[0, 50, 150, 300].map((j) => (
              <option key={j} value={j}>
                jitter {j} ms
              </option>
            ))}
          </select>
          <select value={corrupt} onChange={(e) => setCorrupt(Number(e.target.value))} title="corruption">
            {[0, 0.01, 0.05].map((c) => (
              <option key={c} value={c}>
                corrupt {c * 100}%
              </option>
            ))}
          </select>
          <button onClick={() => (mockNetwork.stalledUntil = performance.now() + 3000)}>Stall 3 s</button>
        </Group>
        <Group label="Lifecycle">
          <label>
            <input type="checkbox" checked={mounted} onChange={(e) => setMounted(e.target.checked)} /> mounted
          </label>
          <label>
            <input type="checkbox" checked={spacer} onChange={(e) => setSpacer(e.target.checked)} /> push off-screen
          </label>
          <button onClick={takeSnapshot}>Snapshot</button>
        </Group>
        <span style={{ opacity: 0.7 }}>
          open streams: <b data-testid="open-streams">{mockStreamCounters.open}</b> (opened {mockStreamCounters.opened})
        </span>
      </header>

      {spacer && <div style={{ height: "150vh", display: "grid", placeItems: "center" }}>scroll down ↓</div>}

      {mounted && (
        <div style={{ ...gridStyle, gridTemplateColumns: grid ? "1fr 1fr" : "1fr" }}>
          {cameras.map((cameraId, index) => (
            <Tile
              key={index}
              index={index}
              title={cameraId}
              source={sourceFor(cameraId)}
              // Multi-view: only the first tile can be heard.
              muted={index === 0 ? muted : true}
              fit={fit}
              handle={index === 0 ? mainTile : undefined}
              onTime={index === 0 ? (ms) => (currentTime.current = ms) : undefined}
            />
          ))}
        </div>
      )}

      {snapshotUrl && (
        <figure style={{ margin: 12 }}>
          <img src={snapshotUrl} style={{ maxWidth: 320, border: "1px solid #334" }} alt="snapshot" />
          <figcaption style={{ fontSize: 12, opacity: 0.8 }}>{snapshotInfo}</figcaption>
        </figure>
      )}
      {!snapshotUrl && snapshotInfo && <p style={{ margin: 12 }}>{snapshotInfo}</p>}
    </div>
  );
}

function Tile(props: {
  index: number;
  title: string;
  source: StreamRequest;
  muted: boolean;
  fit: "contain" | "cover";
  handle?: Ref<VideoSurfaceHandle>;
  onTime?: (ms: number) => void;
}) {
  const report = useRef<TileReport>({ state: { kind: "idle" }, history: [] });
  reports[props.index] = report.current;
  const { state, stats, timeMs } = report.current;
  return (
    <div style={tile}>
      <VideoSurface
        ref={props.handle}
        source={props.source}
        muted={props.muted}
        fit={props.fit}
        onState={(s) => {
          report.current.state = s;
          report.current.history = [...report.current.history.slice(-30), [Math.round(performance.now()), s.kind]];
        }}
        onStats={(s) => (report.current.stats = s)}
        onTime={(ms) => {
          report.current.timeMs = ms;
          props.onTime?.(ms);
        }}
      />
      <div style={overlay} data-testid={`tile-${props.index}`}>
        <div>
          <b>{props.title}</b> · {state.kind}
          {state.kind === "error" && (
            <span style={{ color: "#ff8a80" }}>
              {" "}
              {state.code}: {state.message}
            </span>
          )}
          {!props.muted && " · 🔊"}
        </div>
        <div>{timeMs !== undefined ? new Date(timeMs).toISOString() : "—"}</div>
        {stats && (
          <div>
            {stats.width}×{stats.height} {stats.codec} · {stats.fps} fps · {(stats.bitrate / 1000).toFixed(0)} kb/s ·
            latency {stats.latencyMs ?? "—"} ms (target {stats.targetDelayMs}) · buffer {stats.bufferMs} ms · dropped{" "}
            {stats.droppedFrames} · queue {stats.decodeQueue} · decode {stats.decodeMs ?? "—"} ms · clock {stats.clock}
            {stats.audioOffsetMs !== undefined && ` (audio ${stats.audioOffsetMs} ms)`}
          </div>
        )}
      </div>
    </div>
  );
}

function Group({ label, children }: { label: string; children: ReactNode }) {
  return (
    <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
      <span style={{ opacity: 0.6 }}>{label}</span>
      {children}
    </span>
  );
}

const page: CSSProperties = {
  fontFamily: "system-ui, sans-serif",
  fontSize: 13,
  background: "#05070a",
  color: "#dfe6ee",
  minHeight: "100vh",
  margin: -8,
};
const toolbar: CSSProperties = {
  display: "flex",
  flexWrap: "wrap",
  gap: 16,
  alignItems: "center",
  padding: "10px 12px",
  borderBottom: "1px solid #1d2530",
  position: "sticky",
  top: 0,
  background: "#0b0f15",
  zIndex: 1,
};
const gridStyle: CSSProperties = { display: "grid", gap: 8, padding: 8 };
const tile: CSSProperties = { position: "relative", aspectRatio: "16 / 9", background: "#000" };
const overlay: CSSProperties = {
  position: "absolute",
  left: 0,
  bottom: 0,
  right: 0,
  padding: "4px 8px",
  font: "12px ui-monospace, monospace",
  background: "rgba(0,0,0,0.55)",
  pointerEvents: "none",
};

// HMR may run this module again: keep one root.
const host = window as unknown as { __harnessRoot?: ReturnType<typeof createRoot> };
host.__harnessRoot ??= createRoot(document.getElementById("root") as HTMLElement);
host.__harnessRoot.render(
  <StrictMode>
    <Harness />
  </StrictMode>,
);
