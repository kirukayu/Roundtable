// Time the answer cleaner as the text it is handed grows.
//
// This exists because the cleaner was quadratic and nothing noticed for
// months. `settled()` walked the buffer a character at a time and asked its
// questions with `held.slice(at)`, which copies everything from `at` to the
// end — so the work grew with the square of the answer's length. Streaming was
// safe, because pieces arrive small and are released constantly. The plain
// `/chat` path hands the whole answer over in one call, and a model that wrote
// tens of thousands of characters took the worker over Cloudflare's CPU limit:
// `outcome: exceededCpu`, `cpuTime: 32500ms`, against 8-23ms for a request
// that answered. The launcher reported that to the player as a 503.
//
//   node bench/cleaner.mjs
//
// Doubling the input should roughly double the time. If a doubling ever
// quadruples it, the slicing is back.

import { clean } from "../src/index.js";

// Prose with the things the cleaner actually looks at — emphasis, a heading, a
// bullet, a table row, a link — so the walk does real work rather than running
// down a corridor of plain characters.
const PARAGRAPH = [
  "## The Roundtable Hold",
  "",
  "Reduvia is a **bleed dagger** and its skill is *Blood Blade*.",
  "It scales with `arcane` and asks for 13 faith.",
  "- Its fire damage reads 82 in the table.",
  "| weapon | weight | bleed |",
  "See [the notes](https://example.invalid/notes) for the rest.",
  "",
].join("\n");

// The shape that actually hurt. Marked-up text is released in small pieces —
// every heading and bullet is a point where the walk stops and hands something
// over, so the buffer never grows and the old code looked fine. Unbroken prose
// gives it nothing to stop at, so it walks the whole length, and THAT is where
// copying the remainder once per character turns into the CPU limit. Most of a
// real answer is exactly this: sentences.
const PROSE =
  "The Tarnished came to the Roundtable Hold and found it quiet. " +
  "A weapon scales with an attribute and the letter on the screen is a number underneath. " +
  "Bleed builds up until it does not, and then it takes a tenth of what you have. ";

function timeOnce(chars, shape = PARAGRAPH) {
  let text = "";
  while (text.length < chars) text += shape;
  text = text.slice(0, chars);

  const began = process.hrtime.bigint();
  const tidy = clean();
  const out = tidy.take(text) + tidy.rest();
  const took = Number(process.hrtime.bigint() - began) / 1e6;
  return { took, out };
}

// Warm the JIT, so the first size is not paying for compilation.
timeOnce(4_000);

for (const [name, shape] of [["marked up", PARAGRAPH], ["unbroken prose", PROSE]]) {
  console.log(`\n  ${name}\n  chars      ms     ms/char   vs previous`);
  let last = null;
  for (const size of [5_000, 10_000, 20_000, 40_000, 80_000]) {
    const { took } = timeOnce(size, shape);
    const ratio = last === null ? "" : `x${(took / last).toFixed(1)}`;
    console.log(
      `  ${size.toLocaleString().padStart(7)}  ${took.toFixed(1).padStart(6)}` +
        `  ${(took / size * 1000).toFixed(3).padStart(8)}   ${ratio}`,
    );
    last = took;
  }
}
console.log("\n  doubling the input should give about x2. x4 means it is quadratic again.");

// And it must still do its job — a fast cleaner that stopped cleaning would
// pass the timing above and be worse than the bug.
const { out } = timeOnce(4_000);
const problems = [];
if (out.includes("**")) problems.push("bold markers survived");
if (out.includes("##")) problems.push("a heading survived");
if (out.includes("](http")) problems.push("a link target survived");
if (!out.includes("Reduvia")) problems.push("the actual words did not survive");
if (!out.includes("Blood Blade")) problems.push("emphasis ate the words inside it");
console.log(problems.length ? `\n  BROKEN: ${problems.join(", ")}` : "\n  and it still cleans.");
