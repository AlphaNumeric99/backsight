import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import { shouldIgnoreHotkey, usePlaybackHotkeys } from "./use-playback-hotkeys";

function Harness({ onAction, enabled = true }: { onAction: (a: unknown) => void; enabled?: boolean }) {
  usePlaybackHotkeys(enabled, onAction);
  return (
    <div>
      <input aria-label="text" />
      <button type="button">btn</button>
      <div role="tablist">
        <button type="button" role="tab">tab</button>
      </div>
      <div data-playback-timeline="" role="slider" tabIndex={0} aria-label="timeline" aria-valuenow={0} />
    </div>
  );
}

const press = (target: Element | Window, key: string, init: KeyboardEventInit = {}) =>
  target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));

describe("playback hotkeys", () => {
  it("maps keys to actions page-wide", () => {
    const onAction = vi.fn();
    render(<Harness onAction={onAction} />);
    press(window, " ");
    press(window, "ArrowLeft", { shiftKey: true });
    press(window, "]");
    press(window, "L");
    expect(onAction.mock.calls.map((c) => c[0])).toEqual([
      { type: "toggle-play" },
      { type: "seek-by", ms: -60_000 },
      { type: "speed", dir: 1 },
      { type: "live" },
    ]);
  });

  it("works on the timeline, which is a slider", () => {
    const onAction = vi.fn();
    const { getByLabelText } = render(<Harness onAction={onAction} />);
    press(getByLabelText("timeline"), "ArrowRight");
    press(getByLabelText("timeline"), "Home");
    expect(onAction).toHaveBeenCalledTimes(2);
  });

  it("stays out of text fields, composite widgets and native button activation", () => {
    const onAction = vi.fn();
    const { getByLabelText, getByText } = render(<Harness onAction={onAction} />);
    press(getByLabelText("text"), " ");
    press(getByLabelText("text"), "ArrowLeft");
    press(getByText("tab"), "ArrowRight");
    press(getByText("btn"), " ");
    expect(onAction).not.toHaveBeenCalled();
    // Arrows still seek from an ordinary button.
    press(getByText("btn"), "ArrowRight");
    expect(onAction).toHaveBeenCalledTimes(1);
  });

  it("can be switched off (e.g. while clipping)", () => {
    const onAction = vi.fn();
    render(<Harness onAction={onAction} enabled={false} />);
    press(window, " ");
    expect(onAction).not.toHaveBeenCalled();
  });

  it("ignores non-elements", () => {
    expect(shouldIgnoreHotkey(null, { type: "toggle-play" })).toBe(false);
  });
});
