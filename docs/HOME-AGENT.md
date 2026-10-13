# The home agent

> **Status: a design, not an implementation.** Nothing here is built yet. It is written down
> so the decisions are arguable before any of it is code.

Backsight today runs on one machine and talks to the cameras itself. That works beautifully
on the sofa and not at all from a train. This describes the piece that fixes it: a small
always-on program at home that the clients talk to instead.

## What it has to satisfy

- **No VPN on the client.** Not "a VPN you leave on", not "a VPN that connects on demand" —
  none. A phone that has to hold a VPN profile for a camera app has already lost, and on iOS
  it conflicts with the one profile people actually need for work.
- **No machine kept switched on for it.** A desktop that must stay awake is not a design,
  it's a tax.
- **Nobody in the middle who can watch.** The point of this project is not depending on a
  vendor's cloud. Replacing TP-Link's servers with someone else's is not progress.
- **No listener on the public internet at home.** Cameras must never be exposed, and neither
  should the agent.

## The shape

```
  cameras ──LAN── agent (always-on box at home)
                     │
                     ╰── outbound only ──► rendezvous ◄── clients (phone, laptop)
```

The agent holds the camera credentials and speaks the camera protocols. Clients speak only
to the agent. Nothing at home accepts an inbound connection: the agent dials out to a
rendezvous point and the clients meet it there.

On the local network a client can still talk to the cameras directly, exactly as the desktop
app does now — that path is lower latency and depends on nothing, so it stays.

### Why an agent rather than a port forward

A forwarded port puts a camera's firmware — the thing whose certificate handling we have
already had to work around — on the public internet, where it is scanned within hours.
An agent is one program, with one door, that we control and can revoke access to. It also
happens to solve problems a tunnel doesn't: credentials never reach the phone, the camera
protocols are spoken once rather than on every client, and a device that is always on can
notice things while you are asleep.

## Trust

- **Camera passwords never leave the agent.** A client is enrolled with its own key and has
  no idea what the TP-Link password is.
- **Enrolment happens on the local network**, by scanning a code shown by the agent. Not over
  the internet, not by typing an account password into a phone.
- **Each device is separately revocable.** A lost phone is one line removed, not a password
  change propagated to every camera.
- **The rendezvous is assumed hostile.** See below for what that means per transport.

## Two transports

The agent offers an ordinary HTTPS API — cameras, status, events, thumbnails, clip
downloads — and, for live video, a WebRTC peer. They are deliberately separable: the API is
small and cacheable, live video is large and latency-sensitive, and they want different
things from the network.

### Stage 1 — HTTPS through a rendezvous you own

A small VPS, a WireGuard link from the agent to it, and a reverse proxy. The VPS is the only
public listener anywhere in the system. Clients are plain HTTPS clients, so this needs no
new client technology at all, and live video can start as HTTP streaming.

The honest cost: TLS terminates on the VPS, so it is a machine that *could* watch the video.
It's yours, which is a great deal better than a vendor's, but it is not zero. Every frame
also crosses it, so its bandwidth is the ceiling.

### Stage 2 — WebRTC for live video

WebRTC is built for precisely this problem: two endpoints behind NAT, no VPN, media
encrypted end to end by DTLS-SRTP. The rendezvous shrinks to a signalling service passing a
few kilobytes of text, plus a TURN relay for the connections that fail to go direct. Because
the media keys never leave the endpoints, **the relay cannot watch the cameras even though
the packets pass through it** — the property stage 1 lacks.

Two endpoints behind NAT cannot find each other with nothing in between; that is true of
Tailscale, of Cloudflare and of the vendor clouds too. The question is only how small and
how powerless that middle can be made, and this is about as small as it goes.

## What the hardware decides

The expected home for this is a Raspberry Pi or similar — something already on. That rules
things out, and it is better to say so now:

- **The agent never transcodes.** A Pi-class board cannot re-encode 2K video, so it relays
  the camera's existing stream untouched. This is also the lowest-latency design, and close
  to what the client already does.
- **Remote live view therefore uses the camera's H.264 stream.** H.264 is mandatory in
  WebRTC and hardware-decoded everywhere; H.265 support across browsers is patchy enough to
  be unusable as a baseline. Cameras that only offer H.265 — Qubo, today — cannot do remote
  live through this path without transcoding somewhere.
- **Wired, not wireless**, and **no video written to the SD card.** Recordings stay on the
  cameras' own cards; the agent keeps only small state.

## What the clients become

The UI does not change. It already talks to a `BacksightApi` interface with two
implementations behind it — the desktop backend and the in-memory mock used by `npm run dev`
— so an implementation that speaks HTTPS to the agent drops into the same seam.

The parts that genuinely need work are the live player, which becomes a video element fed by
WebRTC rather than the current custom pipeline, and a touch and small-screen pass over a UI
that was designed for a mouse.

A useful consequence: the agent can serve the UI itself. A phone that opens the agent's
address gets the same application, which means the mobile story can begin without an app
store, a signing certificate or an install.

## What this makes possible that a tunnel doesn't

Something that is always on can watch. Detection events can be noticed as they happen and
pushed to a phone, which is what people actually want from a camera when they are out —
not a live stream they must remember to open. That needs no tunnel and no live video at all,
and it is probably the most valuable thing on this page.

## Open questions

- Whether hole punching succeeds on the ISPs people actually have, which decides whether
  TURN is a rare fallback or the common path — and therefore what running this costs.
- Whether the agent should cache recent clips, trading SD-card wear for instant playback.
- Whether enrolment should survive the agent being reinstalled, and what the recovery story
  is when it doesn't.
