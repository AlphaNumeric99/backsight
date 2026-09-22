// The playback position, shared between the player, the timeline and the overlays without
// re-rendering the page. The player reports time ~10× per second; readers extrapolate between
// reports so the ruler scrolls smoothly at display refresh rate.

import { useSyncExternalStore } from "react";
import { createStore, type StoreApi } from "zustand/vanilla";

export interface ClockState {
  /** Last known position, epoch ms. */
  timeMs: number;
  /** `performance.now()` when `timeMs` was reported. */
  stamp: number;
  /** Whether the position is advancing (playing and not stalled). */
  playing: boolean;
  speed: number;
  /** Increments on every explicit seek, so views can animate to the new position. */
  seekSeq: number;
}

export type PlaybackClock = StoreApi<ClockState> & {
  /** Position reported by the player. */
  report(timeMs: number): void;
  /** An explicit jump (click, key, event card). */
  seek(timeMs: number): void;
  setPlaying(playing: boolean): void;
  setSpeed(speed: number): void;
  /** Current position, extrapolated from the last report. */
  now(at?: number): number;
};

/** Never extrapolate further than this past the last report (a stalled stream stops the ruler). */
const MAX_EXTRAPOLATION_MS = 450;

export function extrapolate(state: ClockState, at: number): number {
  if (!state.playing) return state.timeMs;
  const elapsed = Math.min(Math.max(0, at - state.stamp), MAX_EXTRAPOLATION_MS);
  return state.timeMs + elapsed * state.speed;
}

export function createClock(initialMs: number, speed = 1): PlaybackClock {
  const store = createStore<ClockState>(() => ({
    timeMs: initialMs,
    stamp: performance.now(),
    playing: false,
    speed,
    seekSeq: 0,
  }));
  return Object.assign(store, {
    report(timeMs: number) {
      store.setState({ timeMs, stamp: performance.now() });
    },
    seek(timeMs: number) {
      store.setState((s) => ({ timeMs, stamp: performance.now(), seekSeq: s.seekSeq + 1 }));
    },
    setPlaying(playing: boolean) {
      const s = store.getState();
      if (s.playing === playing) return;
      // Freeze the extrapolated position when stopping, restart the stamp when starting.
      store.setState({ playing, timeMs: extrapolate(s, performance.now()), stamp: performance.now() });
    },
    setSpeed(next: number) {
      const s = store.getState();
      store.setState({ speed: next, timeMs: extrapolate(s, performance.now()), stamp: performance.now() });
    },
    now(at = performance.now()) {
      return extrapolate(store.getState(), at);
    },
  });
}

/**
 * Subscribes a component to the clock at a coarse granularity (default: whole seconds), so a
 * timestamp overlay re-renders once per second of footage instead of every report.
 */
export function useClockTime(clock: PlaybackClock, granularityMs = 1000): number {
  return useSyncExternalStore(
    clock.subscribe,
    () => Math.floor(clock.getState().timeMs / granularityMs) * granularityMs,
  );
}

/** Selects a derived value from the clock; re-renders only when it changes. */
export function useClockSelector<T>(clock: PlaybackClock, select: (timeMs: number) => T): T {
  return useSyncExternalStore(clock.subscribe, () => select(clock.getState().timeMs));
}
