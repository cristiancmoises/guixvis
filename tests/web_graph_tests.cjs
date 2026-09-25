/* Run with: node --test tests/web_graph_tests.cjs */
const test = require("node:test");
const assert = require("node:assert/strict");
const { GraphEngine, GraphCanvas, radiusOf } = require("../web/graph.js");

function engine(options = {}) {
  return new GraphEngine(
    [
      { name: "emacs", degree: 2, depth: 0 },
      { name: "glibc", degree: 1, depth: 1 },
      { name: "bash", degree: 1, depth: 1 },
    ],
    [{ from: "emacs", to: "glibc" }, { from: "emacs", to: "bash" }],
    { fresh: true, ...options }
  );
}

function canvas() {
  const context = { clearRect() {}, setTransform() {} };
  const view = new GraphCanvas({
    getContext: () => context,
    addEventListener() {},
  });
  view.resize(600, 400, 1);
  view.paint = () => {};
  view.drawSkeleton = () => {};
  return view;
}

function interactiveCanvas(options = {}) {
  const listeners = new Map();
  const calls = [];
  const context = new Proxy({
    measureText: (text) => { calls.push(["measureText", text]); return { width: [...text].length * 7 }; },
    createRadialGradient: () => ({ addColorStop() {} }),
  }, { get: (obj, key) => key in obj ? obj[key] : (...args) => calls.push([key, ...args]) });
  const view = new GraphCanvas({
    getContext: () => context, style: {},
    addEventListener: (name, fn) => listeners.set(name, fn),
    getBoundingClientRect: () => ({ left: 0, top: 0 }),
    setPointerCapture() {},
  }, options);
  view.resize(600, 400, 1);
  const emit = (type, values = {}) => listeners.get(type)({
    pointerId: 1, button: 0, buttons: type === "pointerup" ? 0 : 1,
    clientX: 300, clientY: 200, preventDefault() {}, ...values,
  });
  return { view, emit, calls };
}

test("only primary gestures pick or drag; context menu requests one back", () => {
  const picked = [], back = [];
  const { view, emit } = interactiveCanvas({ onPick: (name) => picked.push(name), onBack: () => back.push(true) });
  const layout = new GraphEngine([{ name: "root", degree: 1 }], []);
  view.setGraph(layout);
  layout.pos.set("root", { x: 0, y: 0 });
  for (const button of [1, 2]) {
    emit("pointerdown", { button });
    emit("pointermove", { button, clientX: 330 });
    emit("pointerup", { button, clientX: 330 });
    assert.deepEqual(layout.pos.get("root"), { x: 0, y: 0 });
    emit("pointerdown", { button });
    emit("pointerup", { button });
  }
  assert.deepEqual(picked, []);
  emit("contextmenu", { button: 2 });
  assert.equal(back.length, 1);
  emit("pointerdown");
  emit("pointerup");
  assert.deepEqual(picked, ["root"]);
  emit("pointerdown");
  emit("pointermove", { clientX: 340 });
  emit("pointerup", { clientX: 340 });
  assert.ok(layout.pos.get("root").x > 0);
  assert.equal(picked.length, 1);
});

test("mouse button chords stop dragging when the primary button is released", () => {
  const picked = [];
  const { view, emit } = interactiveCanvas({ onPick: (name) => picked.push(name) });
  const layout = new GraphEngine([{ name: "root", degree: 1 }], []);
  view.setGraph(layout);
  layout.pos.set("root", { x: 0, y: 0 });
  emit("pointerdown", { buttons: 1 });
  // Browsers report intermediate chord presses/releases as pointermove.
  emit("pointermove", { button: 2, buttons: 3 });
  emit("pointermove", { button: 0, buttons: 2 });
  emit("pointermove", { button: -1, buttons: 2, clientX: 340 });
  emit("pointerup", { button: 2, buttons: 0, clientX: 340 });
  emit("pointermove", { button: -1, buttons: 0, clientX: 380 });
  assert.deepEqual(layout.pos.get("root"), { x: 0, y: 0 });
  assert.equal(layout.pinned.size, 0);
  assert.deepEqual(picked, []);
});

test("final non-primary release always cleans up a tracked pointer without following", () => {
  const picked = [];
  const { view, emit } = interactiveCanvas({ onPick: (name) => picked.push(name) });
  const layout = new GraphEngine([{ name: "root", degree: 1 }], []);
  view.setGraph(layout);
  layout.pos.set("root", { x: 0, y: 0 });
  emit("pointerdown", { buttons: 1 });
  emit("pointerup", { button: 2, buttons: 0 });
  assert.equal(layout.pinned.size, 0);
  emit("pointermove", { button: -1, buttons: 0, clientX: 380 });
  assert.deepEqual(layout.pos.get("root"), { x: 0, y: 0 });
  assert.deepEqual(picked, []);
});

test("untracked pointer release preserves a tracked touch long press", async () => {
  const actions = [];
  const { view, emit } = interactiveCanvas({
    onNodeAction: (name, action) => actions.push([name, action]),
  });
  const layout = new GraphEngine([{ name: "root", degree: 1 }], []);
  view.setGraph(layout);
  layout.pos.set("root", { x: 0, y: 0 });

  emit("pointerdown", { pointerType: "touch", pointerId: 1 });
  assert.equal(layout.pinned.has("root"), true);
  emit("pointerup", { pointerType: "mouse", pointerId: 2, button: 2 });
  await new Promise((resolve) => setTimeout(resolve, 500));
  assert.deepEqual(actions, [["root", "tooltip"]]);

  emit("pointerup", { pointerType: "touch", pointerId: 1 });
  assert.equal(layout.pinned.size, 0);
});

test("rectangles share bounded cached label geometry with corner picking and zoom", () => {
  const { view, calls, emit } = interactiveCanvas();
  const name = "very-long-package-name-".repeat(12);
  const layout = new GraphEngine([{ name, degree: 1 }], [], { reducedMotion: true });
  view.setGraph(layout);
  view.setStyle("rectangles");
  const box = layout.boundsOfWorld(name);
  const p = layout.pos.get(name);
  assert.ok(box.halfWidth / layout.unit <= 100);
  assert.ok(box.halfWidth / layout.unit >= 32);
  assert.equal(layout.pick(p.x + box.halfWidth * 0.99, p.y + box.halfHeight * 0.99, 0), name);
  assert.equal(layout.pick(p.x + box.halfWidth + layout.unit, p.y, 0), null);
  const measurements = calls.filter((c) => c[0] === "measureText").length;
  view.paint();
  view.paint();
  assert.equal(calls.filter((c) => c[0] === "measureText").length, measurements);
  const label = calls.find((c) => c[0] === "fillText")[1];
  assert.ok(label.endsWith("…"));
  assert.equal(layout.node(name).name, name);
  const rect = calls.find((c) => c[0] === "rect");
  assert.equal(rect[3], box.halfWidth * 2);
  emit("wheel", { deltaY: -200 });
  assert.deepEqual(layout.boundsOfWorld(name), box);
  const corner = view.toWorld((p.x + box.halfWidth * 0.99) * view.scale + view.tx,
    (p.y + box.halfHeight * 0.99) * view.scale + view.ty);
  assert.equal(layout.pick(corner.x, corner.y, 0), name);
});

test("dense rectangles settle without squeezing their separated extent back into the frame", () => {
  const { view } = interactiveCanvas();
  const nodes = Array.from({ length: 80 }, (_, i) => ({ name: `long-package-name-${i}`, degree: 1 }));
  const layout = new GraphEngine(nodes, [], { reducedMotion: true });
  view.setGraph(layout);
  view.setStyle("rectangles");
  for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) {
    const a = nodes[i].name, b = nodes[j].name;
    const pa = layout.pos.get(a), pb = layout.pos.get(b);
    const ba = layout.boundsOfWorld(a), bb = layout.boundsOfWorld(b);
    assert.ok(Math.abs(pa.x - pb.x) >= ba.halfWidth + bb.halfWidth ||
      Math.abs(pa.y - pb.y) >= ba.halfHeight + bb.halfHeight, `${a} overlaps ${b}`);
  }
  assert.ok([...layout.pos.values()].some((p) => Math.abs(p.x) > 1.6 || Math.abs(p.y) > 1.6));
  assert.equal(view.frame(), false);
  view.setStyle("bubbles");
  assert.equal(layout.style, "bubbles");
});

test("touch pinch and cancellation do not follow nodes; background pan still works", () => {
  const picked = [];
  let backs = 0;
  const { view, emit } = interactiveCanvas({ onPick: (name) => picked.push(name), onBack: () => backs++ });
  const layout = new GraphEngine([{ name: "root", degree: 1 }], []);
  view.setGraph(layout);
  layout.pos.set("root", { x: 0, y: 0 });
  emit("pointerdown", { pointerType: "touch" });
  emit("pointerdown", { pointerType: "touch", pointerId: 2, clientX: 350 });
  emit("pointerup", { pointerType: "touch", pointerId: 2, clientX: 350 });
  emit("pointerup", { pointerType: "touch" });
  assert.deepEqual(picked, []);
  assert.equal(layout.pinned.size, 0);
  emit("pointerdown", { pointerType: "touch" });
  emit("pointercancel");
  emit("pointerup", { pointerType: "touch" });
  assert.deepEqual(picked, []);
  const tx = view.tx;
  emit("pointerdown", { clientX: 30 });
  emit("pointermove", { clientX: 70 });
  emit("pointerup", { clientX: 70 });
  assert.equal(view.tx, tx + 40);
  emit("contextmenu", { pointerType: "touch" });
  assert.equal(backs, 0, "touch long-press retains the tooltip gesture");
});

test("rectangle separation resolves coincident dense nodes even with a short relaxation budget", () => {
  const { view } = interactiveCanvas();
  const nodes = Array.from({ length: 150 }, (_, i) => ({ name: `long-package-${i}`, degree: 1 }));
  const layout = new GraphEngine(nodes, []);
  view.setGraph(layout);
  view.setStyle("rectangles");
  for (const name of layout.names) layout.pos.set(name, { x: 0, y: 0 });
  layout.separate(2);
  for (let i = 0; i < nodes.length; i++) for (let j = i + 1; j < nodes.length; j++) {
    const a = nodes[i].name, b = nodes[j].name;
    const pa = layout.pos.get(a), pb = layout.pos.get(b);
    const ba = layout.boundsOfWorld(a), bb = layout.boundsOfWorld(b);
    assert.ok(Math.abs(pa.x - pb.x) >= ba.halfWidth + bb.halfWidth ||
      Math.abs(pa.y - pb.y) >= ba.halfHeight + bb.halfHeight, `${a} overlaps ${b}`);
  }
});

test("node and radius lookups preserve values without scanning names", () => {
  const layout = engine();
  layout.names.indexOf = () => { throw new Error("linear lookup"); };
  assert.equal(layout.node("emacs"), layout.nodes[0]);
  assert.equal(layout.node("missing"), null);
  assert.equal(layout.radiusOfWorld("emacs"), radiusOf(2) * layout.unit);
  layout.unit = 0.25;
  assert.equal(layout.radiusOfWorld("emacs"), radiusOf(2) * 0.25);
  assert.doesNotThrow(() => layout.tick());
});

test("settled graph frames stop layout work and animation requests", () => {
  const view = canvas();
  const layout = engine();
  view.setGraph(layout, { root: "emacs" });
  view.loadedAt = -1000;
  let frames = 0;
  while (view.frame()) {
    assert.ok(++frames < 600, "layout must settle within its cooling budget");
  }
  assert.equal(layout.separated, true);
  layout.tick = () => { throw new Error("settled graph ticked again"); };
  assert.equal(view.frame(), false);
  view.selectNode("bash");
  assert.equal(view.frame(), false);
});

test("failed graph loads clear the canvas without dereferencing a missing engine", () => {
  const view = canvas();
  view.setGraph(engine(), {});
  assert.doesNotThrow(() => view.setGraph(null, {}));
  assert.equal(view.rootName, null);
  assert.equal(view.frame(), false);
});

test("reduced motion settles using the viewport and does not animate loading", () => {
  const view = canvas();
  view.reducedMotion = true;
  view.setSkeleton(true);
  assert.equal(view.frame(), false);
  view.reducedMotion = false;
  assert.equal(view.frame(), true);
  const layout = engine({ reducedMotion: true });
  view.setGraph(layout, {});
  assert.equal(layout.unit, 1 / view.fitScale());
  assert.equal(layout.separated, true);
  assert.equal(view.frame(), false);
});

test("user interactions request one fresh frame", () => {
  const view = canvas();
  let invalidations = 0;
  view.onInvalidate = () => { invalidations += 1; };
  view.selectNode("emacs");
  view.clearSelection();
  view.hoverNode("glibc");
  view.clearHover();
  view.setSkeleton(true);
  assert.equal(invalidations, 5);
});
