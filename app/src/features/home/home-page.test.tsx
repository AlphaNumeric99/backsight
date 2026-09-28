import { beforeEach, describe, expect, it } from "vitest";
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { renderWithProviders } from "@/test/utils";
import { useUiStore } from "@/state/ui";
import { HomePage } from "./home-page";

describe("Cameras home", () => {
  beforeEach(() => {
    act(() => useUiStore.setState({ homeGroup: "all" }));
  });

  it("shows a card per camera with status, model and SD usage", async () => {
    renderWithProviders(<HomePage />);
    const card = (await screen.findByRole("link", { name: "Open Front Door" })).closest("article")!;
    expect(within(card).getByText("Online")).toBeInTheDocument();
    expect(within(card).getByText("C520WS")).toBeInTheDocument();
    expect(within(card).getByRole("meter", { name: "SD card" })).toBeInTheDocument();
    expect(screen.getAllByRole("article")).toHaveLength(7);
    expect(screen.getByText("5 of 7 online · 2 need attention")).toBeInTheDocument();
  });

  it("uses a placeholder when there is no snapshot, and explains locked cameras", async () => {
    renderWithProviders(<HomePage />);
    const offline = (await screen.findByRole("link", { name: "Open Backyard" })).closest("article")!;
    expect(offline.querySelector("img")).toBeNull();
    expect(within(offline).getByText(/^Offline · seen/)).toBeInTheDocument();
    const locked = screen.getByRole("link", { name: "Open Side Gate" }).closest("article")!;
    expect(within(locked).getByText(/^Locked — retry in \d+ min$/)).toBeInTheDocument();
  });

  it("filters by group tab", async () => {
    renderWithProviders(<HomePage />);
    await screen.findByRole("link", { name: "Open Front Door" });
    fireEvent.mouseDown(screen.getByRole("tab", { name: /Indoor/ }));
    fireEvent.click(screen.getByRole("tab", { name: /Indoor/ }));
    await waitFor(() => expect(screen.getAllByRole("article")).toHaveLength(3));
    const names = screen.getAllByRole("article").map((a) => within(a).getByRole("heading").textContent);
    expect(names).toEqual(["Living Room", "Nursery", "Garage"]);
  });
});
