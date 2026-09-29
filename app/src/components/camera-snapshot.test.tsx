import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { CameraSnapshot } from "./camera-snapshot";

describe("CameraSnapshot", () => {
  it("falls back to the placeholder when the preview fails, and tries a newer one", () => {
    const { container, rerender } = render(<CameraSnapshot camera={{ name: "Front Door", snapshotUrl: "thumb://a" }} />);
    fireEvent.error(container.querySelector("img")!);
    expect(container.querySelector("img")).toBeNull();
    expect(screen.getByText("No preview yet")).toBeInTheDocument();

    rerender(<CameraSnapshot camera={{ name: "Front Door", snapshotUrl: "thumb://b" }} />);
    expect(container.querySelector("img")?.getAttribute("src")).toBe("thumb://b");
  });

  it("shows the given placeholder label", () => {
    render(<CameraSnapshot camera={{ name: "Front Door" }} placeholderLabel="Getting a preview…" />);
    expect(screen.getByText("Getting a preview…")).toBeInTheDocument();
  });
});
