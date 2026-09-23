/* Synthetic graph-layout benchmark. Run from any directory with Node.js:
   node examples/bench-web.cjs [--compare-ref <git-ref>]
   This measures layout ticks, not browser rendering or request latency. */
"use strict";

const { execFileSync } = require("node:child_process");
const { readFileSync } = require("node:fs");
const { resolve } = require("node:path");
const { performance } = require("node:perf_hooks");
const vm = require("node:vm");

const project = resolve(__dirname, "..");
const samples = 7;
const ticks = 20;
const nodes = Array.from({ length: 128 }, (_, i) => ({
  name: `package-${i}`, degree: 4, depth: i ? 1 : 0,
}));
const edges = nodes.slice(1).map(({ name }) => ({ from: "package-0", to: name }));

function load(source, filename) {
  // Both revisions run in fresh, equivalent VM contexts.
  const context = { module: { exports: {} } };
  vm.runInNewContext(source, context, { filename, timeout: 5000 });
  const Engine = context.module.exports.GraphEngine;
  if (typeof Engine !== "function") throw new Error(`${filename} does not export GraphEngine`);
  return Engine;
}

function measure(Engine) {
  const engine = new Engine(nodes, edges, { fresh: true });
  const start = performance.now();
  for (let i = 0; i < ticks; i++) engine.tick();
  return performance.now() - start;
}

function main() {
  const args = process.argv.slice(2);
  if (args.length && (args.length !== 2 || args[0] !== "--compare-ref")) {
    throw new Error("Usage: node examples/bench-web.cjs [--compare-ref <git-ref>]");
  }
  const variants = [];
  if (args.length) {
    const options = { cwd: project, encoding: "utf8", maxBuffer: 4 * 1024 * 1024 };
    const commit = execFileSync("git", ["rev-parse", "--verify", "--end-of-options", `${args[1]}^{commit}`], options).trim();
    const source = execFileSync("git", ["show", `${commit}:web/graph.js`], options);
    variants.push({ label: args[1], commit, Engine: load(source, `${commit}:web/graph.js`), times: [] });
  }
  variants.push({
    label: "working tree",
    Engine: load(readFileSync(resolve(project, "web/graph.js"), "utf8"), "web/graph.js"),
    times: [],
  });

  for (const variant of variants) measure(variant.Engine);
  // Alternate baseline/current within each round when comparing revisions.
  for (let round = 0; round < samples; round++) {
    for (const variant of variants) variant.times.push(measure(variant.Engine));
  }
  console.log(JSON.stringify({
    workload: { nodes: nodes.length, edges: edges.length, ticks, samples, warmup_runs: 1, degree_per_node: 4 },
    runtime: { node: process.version, platform: process.platform, arch: process.arch },
    results: variants.map(({ label, commit, times }) => ({
      label, commit,
      median_ms: Number([...times].sort((a, b) => a - b)[Math.floor(samples / 2)].toFixed(2)),
      samples_ms: times.map((time) => Number(time.toFixed(2))),
    })),
  }, null, 2));
}

try { main(); }
catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
