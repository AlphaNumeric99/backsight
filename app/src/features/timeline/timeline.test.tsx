import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { dayRange, HOUR } from "@/lib/time";
import { createClock } from "@/features/playback/clock";
import { Timeline } from "./timeline";

const day = dayRange("2026-09-29", 120);

function setup() {
  const clock = createClock(day.start + 9 * HOUR);
  const onSeek = vi.fn();
  render(
    <TooltipProvider>
      <Timeline
        day={day}
        offsetMinutes={120}
        segments={[{ start: day.start, end: day.start + 12 * HOUR, kind: "continuous" }]}
        events={[]}
        clock={clock}
        onSeek={onSeek}
      />
    </TooltipProvider>,
  );
  return { clock, onSeek };
}

describe("Timeline", () => {
  it("is an accessible slider over the day", () => {
    setup();
    const slider = screen.getByRole("slider", { name: "Playback position" });
    expect(slider).toHaveAttribute("aria-valuemin", String(day.start));
    expect(slider).toHaveAttribute("aria-valuemax", String(day.end));
    expect(slider).toHaveAttribute("tabindex", "0");
  });

  it("has zoom controls and no 'back to playhead' while following", () => {
    setup();
    expect(screen.getByRole("button", { name: "Zoom in" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Zoom out" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Back to playhead" })).toBeNull();
  });

  it("seeks on click, snapped into recorded footage", () => {
    const { onSeek } = setup();
    const slider = screen.getByRole("slider", { name: "Playback position" });
    fireEvent.pointerDown(slider, { button: 0, pointerId: 1, clientX: 0 });
    fireEvent.pointerUp(slider, { button: 0, pointerId: 1, clientX: 0 });
    expect(onSeek).toHaveBeenCalledTimes(1);
    const t = onSeek.mock.calls[0][0] as number;
    expect(t).toBeGreaterThanOrEqual(day.start);
    expect(t).toBeLessThan(day.start + 12 * HOUR);
  });
});

describe("playback clock", () => {
  it("extrapolates between reports while playing, and freezes when paused", () => {
    const clock = createClock(1_000_000);
    clock.setSpeed(2);
    clock.setPlaying(true);
    const at = clock.getState().stamp + 100;
    expect(clock.now(at)).toBe(1_000_200);
    // Never runs away if reports stop.
    expect(clock.now(at + 10_000)).toBe(1_000_000 + 450 * 2);
    clock.setPlaying(false);
    const frozen = clock.getState().timeMs;
    expect(clock.now(at + 10_000)).toBe(frozen);
    const before = clock.getState().seekSeq;
    clock.seek(5);
    expect(clock.getState()).toMatchObject({ timeMs: 5, seekSeq: before + 1 });
  });
});
