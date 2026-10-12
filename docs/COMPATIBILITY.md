# Which cameras work

Backsight has been built and tested against the cameras listed as **confirmed** below. If your
model isn't here, it may still work — this page is short because the project is new, not
because everything else fails.

**Please [send a report](https://github.com/AlphaNumeric99/backsight/issues/new?template=camera_report.yml)
either way.** Reports are the only way this list grows; nobody has every model.

## The short answer

What decides whether a Tapo camera works is **which sign-in its firmware uses**, not the model
number:

| Sign-in | Works? | Which cameras |
|---|---|---|
| "Secure" (`encrypt_type` 3) | **Yes** | Most Tapo cameras from 2023 onward |
| Newer TP-Link sign-in | **No** | Some 2026 firmware, e.g. C225, C220, C510W, C200 V5 |
| Legacy (pre-2023) | Untested | Older firmware |

Backsight tells you which one your camera uses: open **Add camera** and look at the list. A
camera on the newer sign-in is greyed out with a note, so you don't have to guess or spend a
password attempt finding out.

You also need, on any model:

- **Third-Party Compatibility** turned on in the Tapo app (Me → Third-Party Services).
- An **SD card** in the camera, for playback and events. Live view works without one.
- The **TP-Link account password of the camera's owner** — not a camera-account password.

## Confirmed working

| Model | Hardware | Firmware | Live | Playback | Events | Export | Notes |
|---|---|---|---|---|---|---|---|
| C325WB | 1.0 | 1.4.4 | ✅ | ✅ | ✅ | ✅ | |
| C325WB | 2.0 | 1.2.6, 1.3.3 | ✅ | ✅ | ✅ | ✅ | 1.3.3 serves an X.509 v1 certificate; supported since 0.1.1 |

Tested on Windows. macOS and Linux builds are produced by CI and have had far less testing
against real hardware — reports from those especially welcome.

## Expected to work

Cameras on the "Secure" sign-in that aren't in the table above. Nobody has confirmed them yet:

C100, C110, C120, C200, C210, C220, C225, C320WS, C420, C500, C520WS, TC65, TC70 and similar —
**subject to firmware**. A recent firmware update can move a model to the newer sign-in, which
Backsight can't speak.

## Known not to work

| What | Why |
|---|---|
| Cameras on TP-Link's newer sign-in | The handshake isn't implemented. Shown as "Not supported" when adding. |
| Hubs (H100, H200) | No camera to view. |
| Battery cameras | They sleep, and aren't reachable the same way. Not attempted yet. |
| Cameras without an SD card | Live view works; there is nothing to play back. |

## Things that vary by model

- **H.265 (HEVC)** — the 4K and some 2K models record in H.265. On Windows that needs the free
  "HEVC Video Extensions" from the Microsoft Store. Backsight says so rather than failing
  silently.
- **Pan/tilt** — not implemented yet, on any model.
- **Two-way audio** — not implemented yet.
- **Recording mode and SD status** — a few models don't answer these calls. Backsight carries on
  without that information rather than treating the camera as broken.

## If it doesn't work

1. Check **Third-Party Compatibility** is on in the Tapo app.
2. Check the camera and this computer are on the same network. Backsight finds cameras by
   broadcasting, which some guest networks and VLANs block — you can type the IP address
   instead.
3. Run with logging and look at the real error:

   ```bash
   BACKSIGHT_LOG=debug backsight
   ```

4. [Open a report](https://github.com/AlphaNumeric99/backsight/issues/new?template=camera_report.yml)
   with the model, firmware and what happened. Please remove IP addresses, MAC addresses and
   anything else you'd rather not publish.

## A note on firmware updates

Tapo cameras update themselves. An update regenerates the camera's TLS certificate, and
Backsight pins that certificate when you add a camera — so after an update you may see
**"Security certificate changed"** and the camera will stop connecting.

That is expected, and the camera page explains it and offers to trust the new certificate after
checking your password still works. It is deliberately a decision you make rather than something
the app does quietly, because the same symptom would appear if something were impersonating your
camera.

A firmware update can also change the sign-in generation. If a camera that used to work stops
after an update, that's worth reporting — it tells everyone else what to expect.
