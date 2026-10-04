# Contributing to Backsight

Thanks for taking a look. Backsight is an independent, unofficial project — see the
[disclaimer](README.md#disclaimer).

Bug reports from real cameras are the most useful thing you can send, especially from models
we haven't tried. Patches are welcome too.

## Scope

Backsight talks to cameras **you own**, over their **local network interfaces**, with
**credentials you hold**. That is the line the project works within.

What we don't do, and won't merge:

- Anything that circumvents a device's or account's security rather than authenticating to it
  normally — defeating authentication you lack credentials for, exploiting vulnerabilities, or
  working around subscription or licensing controls.
- Routing through a vendor's cloud service, which ties the project to an undocumented private
  API and its terms of service.
- Code copied from GPL-licensed projects. Backsight is MIT. `python-kasa` in particular is
  GPL-3.0 — behaviour reference only, never copied. Ports from MIT sources are fine with
  attribution in [NOTICE](NOTICE).

Never commit camera footage, screenshots of real cameras, IP addresses, MAC addresses or
credentials. Screenshots for docs come from the demo cameras (`npm run dev`).

## Getting set up

Prerequisites: Rust 1.90+, Node.js 22+, and the
[Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
cd app
npm install
npm run tauri dev
```

You don't need a camera to work on most of this:

- `npm run dev` in `app/` runs the UI in a normal browser against in-memory mock cameras, with
  a fixture video stream through the real player pipeline.
- `cargo run -p tapo-mock -- --count 2` starts fake cameras on `127.0.0.2`, `127.0.0.3`, …
  (password `mock`) that you can add in the real app — they implement the login, the control
  API and the media stream.

## Before you open a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
cargo test --workspace --exclude backsight --doc
cd app && npm test && npx tsc --noEmit -p .
```

CI runs the same thing on Windows, Linux and macOS.

### Why the two test commands

`tauri-build` links the static CRT on Windows. To do that it writes an empty `msvcrt.lib` into
its `OUT_DIR` — overriding the one Rust hard-codes — and passes `/NODEFAULTLIB:msvcrt.lib`
alongside `/DEFAULTLIB:libcmt.lib`. It also adds that directory to the link search path
unconditionally.

Rust 2024 compiles every doctest in a workspace into a single binary. That search path reaches
the doctest link, but the matching `/NODEFAULTLIB` flags do not, so the linker resolves
`msvcrt.lib` to the empty stub and the whole C runtime comes back unresolved. Excluding the
app crate from the doctest run keeps its build-script output off that link line.

## Working on the code

- **Match the surrounding code.** Same naming, same comment density, same idiom.
- **Comments explain why**, not what. Several camera quirks look like bugs until you know the
  reason — keep those explanations near the code.
- **Keep user-facing copy in `app/src/lib/strings.ts`**, so it can be translated later.
- **The wire format is a contract.** `app/src-tauri/src/wire.rs` and `app/src/player/wire.ts`
  share golden test vectors in `app/src/player/__fixtures__/`. Change both, and the vectors.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) covers the contracts, the wire format and the
  camera constraints the backend has to respect.

## Camera constraints worth knowing

These are the ones that bite:

- Control requests go out one at a time per camera.
- **A failed login is never retried automatically.** Ten failures lock the camera for about
  30 minutes, and the app must not lock a user out of their own camera.
- The camera limits SD playback clients; a refusal surfaces as "Playback is busy" and is not
  retried in a loop.
- At most 3 local live viewers per camera, and other software (NVRs, ONVIF clients) counts.
- A camera's clock is not necessarily UTC. Recording and event times are cached in camera-clock
  seconds and converted when served.

## Reporting a bug

Please include your camera model and firmware version, your OS, and what you expected versus
what happened. `BACKSIGHT_LOG=debug` turns on backend logging — **read it before posting** and
remove IP addresses, MAC addresses and anything else you'd rather not publish.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).
