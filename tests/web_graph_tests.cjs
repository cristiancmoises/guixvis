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
