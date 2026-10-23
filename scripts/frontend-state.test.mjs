import { readFile } from "node:fs/promises";
import assert from "node:assert/strict";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/taskState.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext } });
const { decodeEventSlot, decodeCountDelta } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`);
const item = (inputPath, succeeded = true) => ({ inputPath, succeeded });

test("out-of-order batch results and retransmissions preserve slots and counts", () => {
  const paths = Array.from({ length: 1000 }, (_, index) => `/photos/${index}.png`);
  const slots = paths.map((_, index) => index);
  const results = [];
  let completed = 0, succeeded = 0;
  for (const index of [...slots].reverse().concat(slots)) {
    const incoming = item(paths[index], index % 10 !== 0);
    const slot = decodeEventSlot({ taskId: "batch", current: index + 1, item: incoming }, "batch", paths, slots);
    const delta = decodeCountDelta(results[slot], incoming);
    completed += delta.completed; succeeded += delta.succeeded; results[slot] = incoming;
  }
  assert.equal(completed, 1000); assert.equal(succeeded, 900);
  assert.equal(results[999].inputPath, paths[999]);
});

test("deep recovery replaces the original slot and stale events are rejected", () => {
  const incoming = item("/photos/failure.png");
  const event = { taskId: "deep", current: 1, item: incoming };
  assert.equal(decodeEventSlot(event, "deep", [incoming.inputPath], [75]), 75);
  assert.equal(decodeEventSlot(event, "another-task", [incoming.inputPath], [75]), undefined);
  assert.equal(decodeEventSlot({ ...event, current: 2 }, "deep", [incoming.inputPath], [75]), undefined);
  assert.equal(decodeEventSlot(event, "deep", ["/photos/other.png"], [75]), undefined);
  assert.deepEqual(decodeCountDelta(item(incoming.inputPath, false), incoming), { completed: 0, succeeded: 1 });
});
