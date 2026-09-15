// Writes the wire-format golden vectors (src/player/__fixtures__/wire/<name>.bspk) from their
// description in vectors.json, using the TypeScript encoder. Node 22.18+ runs the .ts imports
// directly (type stripping).
//
//   node scripts/make-wire-vectors.mjs
//
// The .bspk files are committed. After changing them, run both suites: `npm test` (decoder and
// encoder) and `cargo test -p backsight` (the Rust encoder must produce the same bytes).

import { readFileSync, writeFileSync } from "node:fs";
import { encodeBatch } from "../src/player/wire.ts";
import { bytesToHex, specToPacket } from "../src/player/__fixtures__/wire/vectors.ts";

const dir = new URL("../src/player/__fixtures__/wire/", import.meta.url);
const { vectors } = JSON.parse(readFileSync(new URL("vectors.json", dir), "utf8"));

for (const vector of vectors) {
  const bytes = encodeBatch(vector.packets.map(specToPacket));
  writeFileSync(new URL(`${vector.name}.bspk`, dir), bytes);
  console.log(`${vector.name}.bspk  ${bytes.length} bytes  ${bytesToHex(bytes.subarray(0, 24))}…`);
}
