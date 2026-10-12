import { describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import type { Camera } from "@/ipc";
import { api } from "@/ipc";
import { renderWithProviders } from "@/test/utils";
import { CertificateNotice } from "./certificate-notice";

const PINNED = "812C753F22137D17CA3CD5FBA3BEB37482020FDD7363E61D52D79D6FCCA16F81";
const ACTUAL = "32D649F8F39B0ABFBACE857AD6DC05A5922C81ADC0C28E8D4BFD87D3109E4FAD";

function camera(): Camera {
  return {
    id: "c1",
    name: "Front Door",
    brand: "tapo",
    host: "192.168.1.20",
    groupIds: [],
    favorite: false,
    hasCameraAccount: false,
    status: {
      state: "certificate_changed",
      pinnedCertificate: PINNED,
      actualCertificate: ACTUAL,
    },
  };
}

describe("CertificateNotice", () => {
  it("shows both fingerprints so they can be compared", async () => {
    renderWithProviders(<CertificateNotice camera={camera()} />);
    expect(await screen.findByText("Pinned when added")).toBeInTheDocument();
    expect(screen.getByText("Presented now")).toBeInTheDocument();
    // Grouped for legibility, so match on the digits rather than the spacing.
    const shown = screen.getByText(/812C/).textContent?.replace(/\s/g, "");
    expect(shown).toBe(PINNED);
  });

  it("explains the benign cause and the one worth checking", async () => {
    renderWithProviders(<CertificateNotice camera={camera()} />);
    expect(await screen.findByText(/firmware update or a factory reset/i)).toBeInTheDocument();
    expect(screen.getByText(/answering in the camera's place/i)).toBeInTheDocument();
  });

  it("re-pins only when the owner asks", async () => {
    const trust = vi.spyOn(api, "trustCertificate").mockResolvedValue({
      ...camera(),
      status: { state: "online" },
    });
    renderWithProviders(<CertificateNotice camera={camera()} />);
    const button = await screen.findByRole("button", { name: /trust the new certificate/i });

    expect(trust).not.toHaveBeenCalled();
    fireEvent.click(button);
    await waitFor(() => expect(trust).toHaveBeenCalledWith("c1"));
    trust.mockRestore();
  });
});
