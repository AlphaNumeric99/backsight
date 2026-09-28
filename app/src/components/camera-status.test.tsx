import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { describeStatus, StatusPill } from "./camera-status";

const NOW = Date.parse("2026-09-29T10:00:00Z");

describe("camera status copy", () => {
  it("counts down a lockout", () => {
    const info = describeStatus({ state: "locked", lockedUntil: "2026-09-29T10:23:00Z" }, NOW);
    expect(info.label).toBe("Locked — retry in 23 min");
    expect(info.tone).toBe("danger");
    expect(info.viewable).toBe(false);
    expect(describeStatus({ state: "locked", lockedUntil: "2026-09-29T09:00:00Z" }, NOW).label).toBe("Locked — retry now");
    expect(describeStatus({ state: "locked", lockedUntil: "2026-09-29T11:30:00Z" }, NOW).label).toBe(
      "Locked — retry in 1 h 30 min",
    );
  });

  it("says when an offline camera was last seen", () => {
    expect(describeStatus({ state: "offline", lastSeen: "2026-09-29T07:47:00Z" }, NOW).label).toBe("Offline · seen 2 h ago");
    expect(describeStatus({ state: "offline" }, NOW).label).toBe("Offline");
  });

  it("covers every state", () => {
    expect(describeStatus({ state: "online" }, NOW)).toMatchObject({ label: "Online", tone: "success", viewable: true });
    expect(describeStatus({ state: "connecting" }, NOW)).toMatchObject({ pulse: true, viewable: true });
    expect(describeStatus({ state: "privacy" }, NOW).label).toBe("Privacy mode");
    expect(describeStatus({ state: "auth_failed" }, NOW).label).toBe("Password rejected");
    expect(describeStatus({ state: "unsupported" }, NOW).label).toBe("Not supported");
  });

  it("renders a pill", () => {
    render(<StatusPill status={{ state: "online" }} />);
    expect(screen.getByText("Online")).toBeInTheDocument();
  });
});
