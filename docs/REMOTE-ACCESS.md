# Watching your cameras from somewhere else

Backsight talks to cameras on your local network. Away from home it won't find them — and that
is on purpose.

The good news: you don't need anything from Backsight to fix this. Put your laptop and your
cameras on the same private network with a mesh VPN, and Backsight works unchanged, because as
far as it is concerned you're at home.

**This page recommends [Tailscale](https://tailscale.com), which is free for personal use.**
WireGuard, ZeroTier or your router's own VPN work the same way.

## What not to do

**Don't forward ports to your cameras.** Giving a camera a public address exposes it to the
whole internet, and these devices are not built for that — one of the cameras this project was
built against serves a 1024-bit certificate that modern TLS libraries refuse outright. Anything
on the internet will find it within hours.

A VPN gives you the same access with none of that exposure.

## Setting it up with Tailscale

You need something at home that stays on: a desktop, a Raspberry Pi, a NAS, or a router that
runs Tailscale.

1. **Make an account** at [tailscale.com](https://tailscale.com) — the free tier covers personal
   use comfortably.
2. **Install it on the always-on machine at home** and sign in.
3. **Advertise your home network** from that machine, so its Tailscale peers can reach your
   cameras. If your cameras are on `192.168.1.x`:

   ```bash
   sudo tailscale up --advertise-routes=192.168.1.0/24
   ```

   On Windows:

   ```powershell
   tailscale up --advertise-routes=192.168.1.0/24
   ```

   Use the range your cameras are actually on — check a camera's IP in the Tapo app or in
   Backsight. `192.168.0.0/24` and `10.0.0.0/24` are also common.
4. **Approve the route** in the [Tailscale admin console](https://login.tailscale.com/admin/machines):
   find the machine, open its menu, and enable the advertised subnet route.
5. **Install Tailscale on the laptop** you want to watch from, and sign in to the same account.
6. **Accept routes on that laptop** so it uses the home subnet:

   ```bash
   sudo tailscale up --accept-routes
   ```

   Windows and macOS accept routes by default.

Now open Backsight on the laptop. Your cameras are reachable at the same addresses as at home.

## A couple of practical notes

- **Automatic discovery won't work remotely.** Backsight finds cameras by broadcasting on the
  local network, and broadcasts don't cross a VPN. Add cameras while you're at home, or type the
  IP address directly — once added, they work from anywhere.
- **Upload speed is the limit.** Live HD is roughly 1–3 Mbps per camera, and that's your home
  connection's *upload*, which is usually the slower direction. If it stutters, switch the
  stream to SD.
- **Exports are a download.** Pulling a clip over a VPN takes as long as your home upload
  allows.

## Why Backsight doesn't do this itself

A built-in relay would mean running servers, holding a path to your cameras, and becoming
responsible for the security of that path. A mesh VPN already solves it properly, is maintained
by people who do only that, and keeps your video going directly between your own machines.

If you've tried this and it didn't fit your situation,
[say so](https://github.com/AlphaNumeric99/backsight/discussions) — that's genuinely useful to
know.
