import { beforeEach, describe, expect, it } from "vitest";
import { act, fireEvent, screen } from "@testing-library/react";
import { renderWithProviders } from "@/test/utils";
import { useUiStore } from "@/state/ui";
import { AddCameraDialog, isValidHost } from "./add-camera-dialog";

async function openAndPick(deviceName: string) {
  act(() => useUiStore.setState({ addCameraOpen: true }));
  renderWithProviders(<AddCameraDialog />);
  const row = await screen.findByRole("button", { name: new RegExp(`^${deviceName}`) });
  fireEvent.click(row);
  return screen.findByLabelText("TP-Link account password");
}

async function submit(password: string) {
  fireEvent.change(screen.getByLabelText("TP-Link account password"), { target: { value: password } });
  fireEvent.click(screen.getByRole("button", { name: "Add camera" }));
}

describe("Add camera dialog", () => {
  beforeEach(() => {
    act(() => useUiStore.setState({ addCameraOpen: false }));
  });

  it("lists discovered devices, with added cameras and hubs disabled", async () => {
    act(() => useUiStore.setState({ addCameraOpen: true }));
    renderWithProviders(<AddCameraDialog />);
    expect(await screen.findByText("11 devices found")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Kitchen/ })).toBeEnabled();
    expect(screen.getByRole("button", { name: /^Front Door/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Tapo Hub/ })).toBeDisabled();
  });

  it("requires the password", async () => {
    await openAndPick("Kitchen");
    fireEvent.click(screen.getByRole("button", { name: "Add camera" }));
    expect(await screen.findByText("Enter the TP-Link account password.")).toBeInTheDocument();
  });

  it("explains a rejected password", async () => {
    await openAndPick("Kitchen");
    await submit("wrong");
    expect(await screen.findByText("Password not accepted")).toBeInTheDocument();
    expect(screen.getByText(/10 failed attempts/)).toBeInTheDocument();
  });

  it("explains Third-Party Compatibility, step by step", async () => {
    await openAndPick("Hallway");
    await submit("compat");
    expect(await screen.findByText("Third-Party Compatibility is off")).toBeInTheDocument();
    expect(screen.getByText("Go to Me → Third-Party Services")).toBeInTheDocument();
  });

  it("counts down a lockout and blocks retries until then", async () => {
    await openAndPick("Doorbell");
    await submit("locked");
    expect(await screen.findByText("Camera is locked")).toBeInTheDocument();
    expect(screen.getByText(/accepts logins again in 23 min/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again in 23 min" })).toBeDisabled();
  });

  it("adds a camera", async () => {
    act(() => useUiStore.setState({ addCameraOpen: true }));
    renderWithProviders(<AddCameraDialog />);
    await screen.findByText("11 devices found");
    fireEvent.change(screen.getByLabelText("Or enter an IP address"), { target: { value: "192.168.1.60" } });
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await screen.findByLabelText("TP-Link account password");
    fireEvent.change(screen.getByLabelText(/^Name/), { target: { value: "Kitchen cam" } });
    await submit("correct horse battery staple");
    expect(await screen.findByText("Kitchen cam is ready")).toBeInTheDocument();
  });

  it("validates manual addresses", () => {
    expect(isValidHost("192.168.1.20")).toBe(true);
    expect(isValidHost("camera.local")).toBe(true);
    expect(isValidHost("300.1.1.1")).toBe(false);
    expect(isValidHost("192.168.1")).toBe(false);
    expect(isValidHost("")).toBe(false);
  });

  it("signs in to Qubo, picks a camera and opens the saved-camera step", async () => {
    act(() => useUiStore.setState({ addCameraOpen: true }));
    renderWithProviders(<AddCameraDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "Add from a Qubo account" }));
    fireEvent.change(await screen.findByLabelText("Qubo account email"), { target: { value: "owner@example.com" } });
    fireEvent.change(screen.getByLabelText("Qubo account password"), { target: { value: "test-password!#'" } });
    fireEvent.click(screen.getByRole("button", { name: "Find my cameras" }));
    const camera = await screen.findByRole("button", { name: /Cam 360 3MP/ });
    expect(screen.queryByLabelText("Qubo account password")).not.toBeInTheDocument();
    fireEvent.click(camera);
    fireEvent.click(screen.getByRole("button", { name: "Add camera" }));
    expect(await screen.findByText("Cam 360 3MP is ready")).toBeInTheDocument();
    expect(screen.getByText(/watch it through Qubo’s cloud/)).toBeInTheDocument();
  });

  it("shows Qubo sign-in errors without Tapo password advice", async () => {
    act(() => useUiStore.setState({ addCameraOpen: true }));
    renderWithProviders(<AddCameraDialog />);
    fireEvent.click(await screen.findByRole("button", { name: "Add from a Qubo account" }));
    fireEvent.change(await screen.findByLabelText("Qubo account email"), { target: { value: "owner@example.com" } });
    fireEvent.change(screen.getByLabelText("Qubo account password"), { target: { value: "wrong" } });
    fireEvent.click(screen.getByRole("button", { name: "Find my cameras" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The Qubo cloud rejected the account.");
    expect(screen.queryByText(/10 failed attempts/)).not.toBeInTheDocument();
  });
});
