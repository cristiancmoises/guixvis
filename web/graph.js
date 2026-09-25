/* guixvis web — graph engine: force-directed layout + canvas rendering.
   Vanilla JS, no dependencies. The GraphEngine is DOM-free (testable);
   GraphCanvas binds it to a <canvas> with DPR scaling and input.

   Coordinate model: engine positions are WORLD units, seeded in a frame
   [-1.6, 1.6] x [-1, 1] (same constants as the TUI layout, which is
   visually validated). The canvas maps world -> pixels with a scale S and
   node radii are converted with unit = 1/S, keeping repulsion, collision
   and label offsets all in one consistent space. */

"use strict";

const COLORS = {
  bg: [10, 12, 16],
  edge: "rgba(120, 134, 160, 0.20)",
  edgeHot: "rgba(94, 196, 255, 0.45)",
  edgeDim: "rgba(120, 134, 160, 0.08)",
  node: [94, 196, 255],
  rootAlt: [186, 120, 255],
  selected: [255, 214, 90],
  label: "rgba(220, 224, 232, 0.95)",
  labelDim: "rgba(220, 224, 232, 0.55)",
  halo: "rgba(94, 196, 255, 0.25)",
};

/* Bubble radius in CSS px: grows logarithmically with degree. */
function radiusOf(degree) {
  return Math.min(22, Math.max(5, 5 + 3.4 * Math.log(1 + degree)));
}

class GraphEngine {
  /* nodes: [{name, degree, depth}]; edges: [{from, to}] (names).
     opts: { fresh, reducedMotion } */
  constructor(nodes, edges, opts = {}) {
    this.nodes = nodes;
    this.edges = edges;
    this.reducedMotion = !!opts.reducedMotion;
    this.names = nodes.map((n) => n.name);
    this.byName = new Map(nodes.map((node) => [node.name, node]));
    this.radii = new Map(nodes.map((node) => [node.name, radiusOf(node.degree)]));
    this.style = "bubbles";
    this.boxes = new Map();
    this.adj = new Map();
    this.names.forEach((n) => this.adj.set(n, []));
    for (const e of edges) {
      if (this.adj.has(e.from) && this.adj.has(e.to)) {
        this.adj.get(e.from).push(e.to);
        this.adj.get(e.to).push(e.from);
      }
    }
    this.pos = new Map();
    this.vel = new Map();
    this.pinned = new Set();
    this.alpha = 1.0;
    this.alphaDecay = opts.fresh ? 0.985 : 0.92;
    this.alphaMin = 0.005;
    /* World units per CSS px; set by the canvas before layout runs. */
    this.unit = 1 / 200;
    this.seedPositions();
  }

  /* Initial placement: reuse previous layout when possible, else a ring. */
  seedPositions(prev) {
    const n = this.names.length;
    if (prev) {
      for (const name of this.names) {
        if (prev.has(name)) {
          this.pos.set(name, { x: prev.get(name).x, y: prev.get(name).y });
        }
      }
    }
    let j = 0;
    for (const name of this.names) {
      if (!this.pos.has(name)) {
        const angle = (j / Math.max(1, n)) * Math.PI * 2 + 0.13;
        const r = 1.2 * Math.sqrt((j + 1) / Math.max(1, n));
        this.pos.set(name, {
          x: Math.cos(angle) * r,
          y: Math.sin(angle) * r,
        });
        j += 1;
      }
      this.vel.set(name, { x: 0, y: 0 });
    }
  }

  pinRoot(name) {
    this.pinned.add(name);
  }

  setPinned(name, on) {
    if (on) this.pinned.add(name);
    else this.pinned.delete(name);
  }

  radiusOfWorld(name) {
    return (this.radii.get(name) || 8) * this.unit;
  }

  boundsOfWorld(name) {
    const box = this.style === "rectangles" && this.boxes.get(name);
    const radius = this.radii.get(name) || 8;
    return {
      halfWidth: (box ? box.width / 2 : radius) * this.unit,
      halfHeight: (box ? box.height / 2 : radius) * this.unit,
    };
  }

  /* Minimum axis displacement for the same boxes used to draw and pick. */
  rectangleOverlap(a, b) {
    const pa = this.pos.get(a), pb = this.pos.get(b);
    const ba = this.boundsOfWorld(a), bb = this.boundsOfWorld(b);
    const dx = pa.x - pb.x, dy = pa.y - pb.y;
    const ox = ba.halfWidth + bb.halfWidth + 8 * this.unit - Math.abs(dx);
    const oy = ba.halfHeight + bb.halfHeight + 8 * this.unit - Math.abs(dy);
    if (ox <= 0 || oy <= 0) return null;
    return ox < oy ? { x: (Math.sign(dx) || 1) * ox, y: 0 }
      : { x: 0, y: (Math.sign(dy) || 1) * oy };
  }

  /* One Fruchterman–Reingold step with radial springs and collision.
     Constants mirror the validated Rust TUI layout (see src/graph.rs). */
  tick() {
    const n = this.names.length;
    if (n === 0) return false;
    const k = 0.18; // constant spacing tuned so equilibrium fits the frame
    const temp = this.alpha;
    const unit = this.unit;

    // Repulsion + collision between every pair.
    for (let i = 0; i < n; i++) {
      const a = this.names[i];
      const pa = this.pos.get(a);
      for (let j = i + 1; j < n; j++) {
        const b = this.names[j];
        const pb = this.pos.get(b);
        let dx = pa.x - pb.x;
        let dy = pa.y - pb.y;
        let d2 = dx * dx + dy * dy;
        let d = Math.sqrt(d2);
        if (d < 1e-4) {
          dx = (i - j) * 0.01;
          dy = (i % 2) * 0.01 - 0.005;
          d2 = dx * dx + dy * dy;
          d = Math.sqrt(d2);
        }
        const force = (k * k) / d;
        const fx = (force * dx) / d;
        const fy = (force * dy) / d;
        this.push(a, fx, fy);
        this.push(b, -fx, -fy);

        if (this.style === "rectangles") {
          const overlap = this.rectangleOverlap(a, b);
          if (overlap) {
            this.push(a, overlap.x / 2, overlap.y / 2);
            this.push(b, -overlap.x / 2, -overlap.y / 2);
          }
          continue;
        }
        // Collision: keep bubbles from overlapping.
        const ra = this.radiusOfWorld(a);
        const rb = this.radiusOfWorld(b);
        const minD = ra + rb + 8 * unit;
        if (d < minD) {
          const push = (minD - d) / 2;
          this.push(a, (dx / d) * push, (dy / d) * push);
          this.push(b, (-dx / d) * push, (-dy / d) * push);
        }
      }
    }

    // Attraction along edges; radial springs anchor hop-1 nodes to the root.
    const root = this.names[0];
    const proot = this.pos.get(root);
    for (const e of this.edges) {
      const a = this.pos.get(e.from);
      const b = this.pos.get(e.to);
      if (!a || !b) continue;
      let dx = a.x - b.x;
      let dy = a.y - b.y;
      const d = Math.sqrt(dx * dx + dy * dy) + 1e-4;
      let w = 0.08;
      if (e.from !== root && e.to !== root) w = 0.045; // cross-edges pull less
      const force = ((d * d) / k) * w;
      this.push(e.from, (-force * dx) / d, (-force * dy) / d);
      this.push(e.to, (force * dx) / d, (force * dy) / d);
    }
    // Radial spring: root <-> direct neighbors.
    for (const nb of this.adj.get(root)) {
      const p = this.pos.get(nb);
      let dx = p.x - proot.x;
      let dy = p.y - proot.y;
      const d = Math.sqrt(dx * dx + dy * dy) + 1e-4;
      const target = 0.55;
      const force = (d - target) * 0.35;
      this.push(nb, (-force * dx) / d, (-force * dy) / d);
    }

    // Soft walls keep the layout inside the initial viewport frame
    // [-1.6, 1.6] x [-1, 1], so the graph is explorable without panning.
    for (const name of this.names) {
      if (this.style === "rectangles") break; // boxes may extend into pannable space
      if (this.pinned.has(name)) continue;
      const p = this.pos.get(name);
      const overX = Math.max(0, Math.abs(p.x) - 1.55);
      const overY = Math.max(0, Math.abs(p.y) - 1.55);
      if (overX > 0) {
        this.push(name, -Math.sign(p.x) * (6 * overX + 10 * overX * overX), 0);
      }
      if (overY > 0) {
        this.push(name, 0, -Math.sign(p.y) * (6 * overY + 10 * overY * overY));
      }
    }

    // Gravity toward centroid, skipping pinned nodes.
    let cx = 0;
    let cy = 0;
    for (const name of this.names) {
      cx += this.pos.get(name).x;
      cy += this.pos.get(name).y;
    }
    cx /= n;
    cy /= n;
    for (const name of this.names) {
      if (this.pinned.has(name)) continue;
      this.push(name, (cx - this.pos.get(name).x) * 0.05, (cy - this.pos.get(name).y) * 0.05);
    }

    // Integrate with temperature cap and velocity damping.
    let moved = 0;
    for (const name of this.names) {
      if (this.pinned.has(name)) {
        this.vel.set(name, { x: 0, y: 0 });
        continue;
      }
      const v = this.vel.get(name);
      v.x *= 0.85;
      v.y *= 0.85;
      const len = Math.sqrt(v.x * v.x + v.y * v.y);
      const cap = temp * 0.25;
      if (len > cap) {
        v.x = (v.x / len) * cap;
        v.y = (v.y / len) * cap;
      }
      const p = this.pos.get(name);
      p.x += v.x;
      p.y += v.y;
      moved += Math.abs(v.x) + Math.abs(v.y);
    }

    this.alpha *= this.alphaDecay;
    return this.alpha > this.alphaMin && moved > 0.15;
  }

  push(name, fx, fy) {
    const v = this.vel.get(name);
    v.x += fx;
    v.y += fy;
  }

  node(name) {
    return this.byName.get(name) || null;
  }

  /* Run until settled (reduced motion / tests). */
  settle(maxTicks = 600) {
    let ticks = 0;
    while (ticks < maxTicks && this.tick()) ticks += 1;
    this.separate();
  }

  /* Hard-constraint pass: push apart every pair whose bubbles overlap.
     Runs after the force layout cools; returns the number of fixes. */
  separate(maxIters = 140) {
    let total = 0;
    for (let iter = 0; iter < maxIters; iter++) {
      let violations = 0;
      for (let i = 0; i < this.names.length; i++) {
        for (let j = i + 1; j < this.names.length; j++) {
          const a = this.names[i];
          const b = this.names[j];
          const pa = this.pos.get(a);
          const pb = this.pos.get(b);
          if (this.style === "rectangles") {
            const overlap = this.rectangleOverlap(a, b);
            if (overlap) {
              violations += 1;
              pa.x += overlap.x / 2;
              pa.y += overlap.y / 2;
              pb.x -= overlap.x / 2;
              pb.y -= overlap.y / 2;
            }
            continue;
          }
          let dx = pa.x - pb.x;
          let dy = pa.y - pb.y;
          let d = Math.hypot(dx, dy);
          const minD = this.radiusOfWorld(a) + this.radiusOfWorld(b) + 0.015;
          if (d < minD) {
            violations += 1;
            if (d < 1e-4) {
              dx = (i - j) * 0.001;
              dy = 0.0005;
              d = Math.hypot(dx, dy);
            }
            const push = (minD - d) / 2;
            pa.x += (dx / d) * push;
            pa.y += (dy / d) * push;
            pb.x -= (dx / d) * push;
            pb.y -= (dy / d) * push;
          }
        }
      }
      total += violations;
      if (violations === 0) break;
    }
    if (this.style === "rectangles") {
      // A finite relaxation budget can leave dense clusters intersecting.
      // Sweep in vertical order to enforce spacing without shrinking any box.
      const ordered = this.names.map((name) => ({
        p: this.pos.get(name), box: this.boundsOfWorld(name),
      })).sort((a, b) => a.p.y - b.p.y);
      for (let i = 0; i < ordered.length; i++) {
        const a = ordered[i];
        for (let j = 0; j < i; j++) {
          const b = ordered[j];
          if (Math.abs(a.p.x - b.p.x) < a.box.halfWidth + b.box.halfWidth + 8 * this.unit) {
            a.p.y = Math.max(a.p.y, b.p.y + a.box.halfHeight + b.box.halfHeight + 8 * this.unit);
          }
        }
      }
    }
    // Shrink back into the frame uniformly (no corner pileups).
    let maxAbs = 0;
    for (const p of this.pos.values()) {
      maxAbs = Math.max(maxAbs, Math.abs(p.x), Math.abs(p.y));
    }
    if (this.style !== "rectangles" && maxAbs > 1.6) {
      const s = 1.6 / maxAbs;
      for (const p of this.pos.values()) {
        p.x *= s;
        p.y *= s;
      }
    }
    this.separated = true;
    return total;
  }

  /* Pick the topmost node at world coords, inflated hit radius. */
  pick(x, y, minRadiusPx = 14) {
    const minR = minRadiusPx * this.unit;
    for (let i = this.names.length - 1; i >= 0; i--) {
      const name = this.names[i];
      const p = this.pos.get(name);
      if (this.style === "rectangles") {
        const box = this.boundsOfWorld(name);
        if (Math.abs(p.x - x) <= Math.max(box.halfWidth, minR) &&
            Math.abs(p.y - y) <= Math.max(box.halfHeight, minR)) return name;
        continue;
      }
      const r = Math.max(this.radiusOfWorld(name), minR);
      const dx = p.x - x;
      const dy = p.y - y;
      if (dx * dx + dy * dy <= r * r) return name;
    }
    return null;
  }
}

/* Canvas binding: world transform, painting, and pointer input. */
class GraphCanvas {
  constructor(canvas, opts = {}) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.onPick = opts.onPick || (() => {});
    this.onBack = opts.onBack || (() => {});
    this.onNodeAction = opts.onNodeAction || (() => {});
    this.onTransform = opts.onTransform || (() => {});
    this.engine = null;
    this.selected = null;
    this.hovered = null;
    this.rootName = null;
    this.scale = 1;
    this.tx = 0;
    this.ty = 0;
    this.w = 0;
    this.h = 0;
    this.skeleton = false;
    this.loadedAt = 0;
    this.dirReverse = false;
    this.style = "bubbles";
    this.labelCache = new Map();
    this.colors = {
      edge: "rgba(120, 134, 160, 0.20)",
      edgeHot: "rgba(94, 196, 255, 0.45)",
      node: [94, 196, 255],
      rootAlt: [186, 120, 255],
      selected: [255, 214, 90],
      label: "rgba(220, 224, 232, 0.95)",
      labelDim: "rgba(220, 224, 232, 0.55)",
      halo: "rgba(94, 196, 255, 0.25)",
      skeleton: "rgba(140, 150, 170, 0.35)",
    };
    this.refreshColors();
    this.bindInput();
  }

  /* Re-read the palette from the CSS variables (theme switching). */
  refreshColors() {
    const read = (name, fallback) => {
      try {
        const v = getComputedStyle(document.documentElement)
          .getPropertyValue(name)
          .trim();
        if (v) return v;
      } catch (_) { /* not in a browser */ }
      return fallback;
    };
    const hexToRgb = (hex, fb) => {
      const m = /^#([0-9a-f]{6})$/i.exec(hex || "");
      if (!m) return fb;
      return [
        parseInt(m[1].slice(0, 2), 16),
        parseInt(m[1].slice(2, 4), 16),
        parseInt(m[1].slice(4, 6), 16),
      ];
    };
    const rgba = (hex, alpha, fb) => {
      const [r, g, b] = hexToRgb(hex, fb);
      return `rgba(${r}, ${g}, ${b}, ${alpha})`;
    };
    const border = read("--border", "#2a3040");
    const fg = read("--fg", "#dce0e8");
    const muted = read("--muted", "#8d94a5");
    const accent = read("--accent", "#5ec4ff");
    const accent2 = read("--accent-2", "#ba78ff");
    const warn = read("--warn", "#ffd65a");
    const glow = parseFloat(read("--graph-glow", "0")) || 0;
    this.colors = {
      edge: rgba(border, 0.35, [120, 134, 160]),
      edgeHot: rgba(accent, 0.5, [94, 196, 255]),
      node: hexToRgb(accent, [94, 196, 255]),
      rootAlt: hexToRgb(accent2, [186, 120, 255]),
      selected: hexToRgb(warn, [255, 214, 90]),
      label: rgba(fg, 0.95, [220, 224, 232]),
      labelDim: rgba(fg, 0.75, [220, 224, 232]),
      halo: rgba(accent, 0.25, [94, 196, 255]),
      skeleton: rgba(muted, 0.35, [140, 150, 170]),
    };
    this.glow = glow;
    this.bgHex = read("--bg", "#0a0c10");
    this.invalidate();
  }

  invalidate() {
    if (this.onInvalidate) this.onInvalidate();
  }

  rectangleLabel(name) {
    if (this.labelCache.has(name)) return this.labelCache.get(name);
    this.ctx.font = "500 12px ui-sans-serif, system-ui, sans-serif";
    const measure = (text) => this.ctx.measureText(text).width;
    let label = name;
    if (measure(label) > 176) {
      const chars = Array.from(name);
      let lo = 0, hi = chars.length;
      while (lo < hi) {
        const mid = Math.ceil((lo + hi) / 2);
        if (measure(chars.slice(0, mid).join("") + "…") <= 176) lo = mid;
        else hi = mid - 1;
      }
      label = chars.slice(0, lo).join("") + "…";
    }
    const box = { label, width: Math.max(64, Math.min(200, measure(label) + 24)), height: 32 };
    this.labelCache.set(name, box);
    return box;
  }

  configureGeometry() {
    if (!this.engine) return;
    this.engine.style = this.style;
    if (this.style === "rectangles") {
      this.engine.boxes = new Map(this.engine.names.map((name) => [name, this.rectangleLabel(name)]));
    }
  }

  setStyle(style) {
    const next = style === "rectangles" ? "rectangles" : "bubbles";
    if (next === this.style) return;
    this.style = next;
    this.configureGeometry();
    if (this.engine) {
      this.engine.alpha = 0.5;
      this.engine.separated = false;
      if (this.engine.reducedMotion) this.engine.settle();
    }
    this.invalidate();
  }

  setGraph(engine, { root, selected, skeleton = false } = {}) {
    this.engine = engine;
    this.rootName = root || (engine ? engine.names[0] : null);
    this.selected = selected || null;
    this.hovered = null;
    this.skeleton = skeleton;
    this.loadedAt = performance.now();
    if (engine) {
      engine.unit = 1 / this.fitScale();
      // Keep measurements only for the active graph, avoiding unbounded history growth.
      this.labelCache = new Map([...this.labelCache].filter(([name]) => engine.byName.has(name)));
      this.configureGeometry();
      if (engine.reducedMotion) engine.settle();
    }
    this.fit();
    this.invalidate();
  }

  setSkeleton(on) {
    this.skeleton = on;
    this.invalidate();
  }

  /* Scale that fits the world frame [-1.6,1.6]x[-1,1] in the viewport. */
  fitScale() {
    return Math.max(1, Math.min(this.w, this.h) / 3.4);
  }

  fit() {
    this.scale = this.fitScale();
    this.tx = this.w / 2;
    this.ty = this.h / 2;
  }

  resize(w, h, dpr) {
    this.w = w;
    this.h = h;
    this.canvas.width = Math.max(1, Math.round(w * dpr));
    this.canvas.height = Math.max(1, Math.round(h * dpr));
    this.ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (this.engine) {
      this.engine.unit = 1 / this.fitScale();
      if (this.style === "rectangles") this.engine.separate();
    }
    if (!this.userTransformed) this.fit();
    this.invalidate();
  }

  /* world <- screen */
  toWorld(sx, sy) {
    return { x: (sx - this.tx) / this.scale, y: (sy - this.ty) / this.scale };
  }

  /* Run layout ticks (up to 4 per frame) and repaint. */
  frame() {
    if (!this.engine || this.skeleton) {
      if (this.skeleton) this.drawSkeleton();
      else this.ctx.clearRect(0, 0, this.w, this.h);
      return this.skeleton && !this.reducedMotion;
    }
    if (!this.engine.reducedMotion && !this.engine.separated) {
      let budget = 4;
      while (budget > 0) {
        budget -= 1;
        if (!this.engine.tick()) {
          this.engine.separate();
          break;
        }
      }
    }
    this.paint();
    return !this.engine.separated || (!this.engine.reducedMotion && performance.now() - this.loadedAt < 180);
  }

  drawSkeleton() {
    const ctx = this.ctx;
    ctx.clearRect(0, 0, this.w, this.h);
    ctx.fillStyle = this.colors.skeleton;
    const t = this.reducedMotion ? 0 : performance.now() / 1000;
    for (let i = 0; i < 26; i++) {
      const a = (i / 26) * Math.PI * 2 + t * 0.4;
      const r = 0.3 + 0.12 * Math.sin(i * 3.7);
      const x = this.w / 2 + Math.cos(a) * r * Math.min(this.w, this.h) * 0.7;
      const y = this.h / 2 + Math.sin(a) * r * Math.min(this.w, this.h) * 0.7;
      ctx.beginPath();
      ctx.arc(x, y, 3 + (i % 4), 0, Math.PI * 2);
      ctx.fill();
    }
  }

  paint() {
    const ctx = this.ctx;
    const eng = this.engine;
    ctx.clearRect(0, 0, this.w, this.h);
    ctx.save();
    ctx.translate(this.tx, this.ty);
    ctx.scale(this.scale, this.scale);

    const fadeIn = eng.reducedMotion ? 1 : Math.min(1, (performance.now() - this.loadedAt) / 180);
    ctx.globalAlpha = fadeIn;
    const unit = eng.unit;

    // Edges (batched).
    ctx.lineWidth = 1 * unit;
    ctx.strokeStyle = this.colors.edge;
    ctx.beginPath();
    for (const e of eng.edges) {
      const a = eng.pos.get(e.from);
      const b = eng.pos.get(e.to);
      if (!a || !b) continue;
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
    }
    ctx.stroke();
    ctx.beginPath();
    ctx.strokeStyle = this.colors.edgeHot;
    const hot = new Set([this.rootName, this.hovered, this.selected].filter(Boolean));
    for (const e of eng.edges) {
      if (hot.has(e.from) || hot.has(e.to)) {
        const a = eng.pos.get(e.from);
        const b = eng.pos.get(e.to);
        if (!a || !b) continue;
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
      }
    }
    ctx.stroke();

    // Nodes.
    for (const name of eng.names) {
      const node = eng.node(name);
      const p = eng.pos.get(name);
      const r = eng.radiusOfWorld(name);
      const isRoot = name === this.rootName;
      const isSel = name === this.selected;
      const isHov = name === this.hovered;
      let rgb = this.colors.node;
      if (isRoot) rgb = this.dirReverse ? this.colors.rootAlt : this.colors.node;
      if (isSel) rgb = this.colors.selected;
      const [cr, cg, cb] = rgb;
      if (eng.style === "rectangles") {
        const box = eng.boundsOfWorld(name);
        ctx.fillStyle = this.bgHex;
        ctx.strokeStyle = `rgb(${cr},${cg},${cb})`;
        ctx.lineWidth = (isSel ? 3 : isRoot || isHov ? 2 : 1) * unit;
        ctx.beginPath();
        ctx.rect(p.x - box.halfWidth, p.y - box.halfHeight, box.halfWidth * 2, box.halfHeight * 2);
        ctx.fill();
        ctx.stroke();
        ctx.font = `500 ${12 * unit}px ui-sans-serif, system-ui, sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillStyle = this.colors.label;
        ctx.fillText(eng.boxes.get(name).label, p.x, p.y);
        continue;
      }
      const grad = ctx.createRadialGradient(
        p.x - r * 0.3, p.y - r * 0.35, r * 0.1, p.x, p.y, r
      );
      grad.addColorStop(
        0,
        `rgba(${Math.min(255, cr + 60)},${Math.min(255, cg + 60)},${Math.min(255, cb + 60)},0.95)`
      );
      grad.addColorStop(1, `rgba(${cr},${cg},${cb},0.9)`);
      const neon = this.glow > 0;
      if (neon || isSel || isHov) {
        ctx.save();
        ctx.shadowColor = neon
          ? `rgba(${cr},${cg},${cb},0.95)`
          : this.colors.halo;
        ctx.shadowBlur = (neon ? this.glow : 18) * unit;
      }
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
      ctx.fill();
      if (neon || isSel || isHov) ctx.restore();
      if (isRoot) {
        ctx.strokeStyle = "rgba(220,224,232,0.85)";
        ctx.lineWidth = 1.5 * unit;
        ctx.beginPath();
        ctx.arc(p.x, p.y, r + 3 * unit, 0, Math.PI * 2);
        ctx.stroke();
      }
      if (isSel && !eng.reducedMotion) {
        ctx.strokeStyle = this.colors.halo;
        ctx.lineWidth = 2 * unit;
        ctx.beginPath();
        ctx.arc(p.x, p.y, r + 5 * unit, 0, Math.PI * 2);
        ctx.stroke();
      }
    }

    if (eng.style === "rectangles") {
      ctx.restore();
      return;
    }
    // Labels are placed in screen space: measured in pixels, checked against
    // the boxes already placed, and drawn with a background-coloured halo so
    // they stay readable where edges pass underneath. Guesswork in world
    // units is what made them collide before.
    const dpr = window.devicePixelRatio || 1;
    const zoom = this.scale;
    const candidates = [];
    const add = (name, priority) => {
      if (name && eng.node(name)) candidates.push({ name, priority });
    };
    add(this.selected, 3);
    add(this.rootName, 3);
    add(this.hovered, 3);
    const wideEnough = zoom >= 0.6 * this.fitScale();
    if (wideEnough) {
      const byDegree = [...eng.names]
        .filter((nm) => !candidates.some((c) => c.name === nm))
        .sort((a, b) => (eng.node(b).degree || 0) - (eng.node(a).degree || 0));
      for (const nm of byDegree.slice(0, 12)) add(nm, 1);
    }

    ctx.save();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.textBaseline = "top";
    const boxes = [];
    const collides = (b) =>
      boxes.some(
        (o) =>
          b.x < o.x + o.w + 3 &&
          b.x + b.w + 3 > o.x &&
          b.y < o.y + o.h + 2 &&
          b.y + b.h + 2 > o.y
      );
    candidates
      .sort((a, b) => b.priority - a.priority)
      .forEach(({ name }) => {
        const node = eng.node(name);
        const p = eng.pos.get(name);
        const r = eng.radiusOfWorld(name);
        const label = name.length > 24 ? name.slice(0, 23) + "…" : name;
        const big = (node.degree || 0) >= 8 || name === this.rootName;
        const size = name === this.rootName ? 13 : big ? 12.5 : 11.5;
        ctx.font = `${name === this.rootName ? "700" : "500"} ${size}px ui-sans-serif, system-ui, sans-serif`;
        const tw = ctx.measureText(label).width;
        const sx = p.x * zoom + this.tx;
        const sy = p.y * zoom + this.ty;
        const rPx = r * zoom;
        const box = { x: sx - tw / 2, y: sy + rPx + 7, w: tw, h: size + 4 };
        if (box.x < 2 || box.x + box.w > this.w - 2) return;
        if (box.y + box.h > this.h - 2) return;
        if (collides(box)) return;
        boxes.push(box);
        ctx.lineWidth = 3;
        ctx.strokeStyle = this.bgHex;
        ctx.strokeText(label, box.x, box.y);
        ctx.fillStyle =
          name === this.rootName || name === this.selected
            ? this.colors.label
            : this.colors.labelDim;
        ctx.fillText(label, box.x, box.y);
      });
    ctx.restore();

    ctx.restore();
  }

  /* ---------- input ---------- */
  bindInput() {
    const c = this.canvas;
    const pointers = new Map();
    let dragNode = null;
    let panning = false;
    let movedTotal = 0;
    let downAt = 0;
    let pinchDist = 0;
    let pinchScale = 1;
    let longPressTimer = null;

    const posOf = (ev) => {
      const rect = c.getBoundingClientRect();
      return { x: ev.clientX - rect.left, y: ev.clientY - rect.top };
    };

    const cancelGesture = () => {
      clearTimeout(longPressTimer);
      pointers.clear();
      if (dragNode && this.engine) this.engine.setPinned(dragNode, false);
      dragNode = null;
      panning = false;
    };

    c.addEventListener("pointerdown", (ev) => {
      if (ev.button !== 0 || this.skeleton) return;
      c.setPointerCapture(ev.pointerId);
      pointers.set(ev.pointerId, posOf(ev));
      movedTotal = 0;
      downAt = performance.now();
      if (pointers.size === 1) {
        const p = posOf(ev);
        const world = this.toWorld(p.x, p.y);
        dragNode = this.engine ? this.engine.pick(world.x, world.y) : null;
        panning = !dragNode;
        if (dragNode) {
          this.engine.setPinned(dragNode, true);
          longPressTimer = setTimeout(() => {
            this.onNodeAction(dragNode, "tooltip", { x: ev.clientX, y: ev.clientY });
          }, 450);
        }
      } else if (pointers.size === 2) {
        clearTimeout(longPressTimer);
        if (dragNode && this.engine) this.engine.setPinned(dragNode, false);
        dragNode = null;
        const [a, b] = [...pointers.values()];
        pinchDist = Math.hypot(a.x - b.x, a.y - b.y) || 1;
        pinchScale = this.scale;
        panning = false;
        this.userTransformed = true;
      }
    });

    c.addEventListener("pointermove", (ev) => {
      const p = posOf(ev);
      // Releasing the primary button during a chord produces pointermove,
      // not pointerup. Stop the gesture as soon as its button is no longer held.
      if (pointers.has(ev.pointerId) && !(ev.buttons & 1)) cancelGesture();
      if (!pointers.has(ev.pointerId)) {
        const world = this.toWorld(p.x, p.y);
        const hover = this.engine ? this.engine.pick(world.x, world.y) : null;
        if (hover !== this.hovered) {
          this.hovered = hover;
          c.style.cursor = hover ? "pointer" : "grab";
          this.invalidate();
        }
        return;
      }
      this.invalidate();
      const prev = pointers.get(ev.pointerId);
      const dx = p.x - prev.x;
      const dy = p.y - prev.y;
      movedTotal += Math.abs(dx) + Math.abs(dy);
      pointers.set(ev.pointerId, p);
      if (Math.abs(dx) + Math.abs(dy) > 8) clearTimeout(longPressTimer);

      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        const d = Math.hypot(a.x - b.x, a.y - b.y) || 1;
        const mid = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        const world = this.toWorld(mid.x, mid.y);
        const s = Math.min(3 * this.fitScale(), Math.max(0.5 * this.fitScale(), pinchScale * (d / pinchDist)));
        this.scale = s;
        this.tx = mid.x - world.x * s;
        this.ty = mid.y - world.y * s;
        return;
      }
      if (pointers.size === 1) {
        if (dragNode && this.engine) {
          const world = this.toWorld(p.x, p.y);
          this.engine.pos.set(dragNode, world);
        } else if (panning) {
          this.tx += dx;
          this.ty += dy;
          this.userTransformed = true;
        } else {
          const world = this.toWorld(p.x, p.y);
          const hover = this.engine ? this.engine.pick(world.x, world.y) : null;
          if (hover !== this.hovered) {
            this.hovered = hover;
            c.style.cursor = hover ? "pointer" : "grab";
          }
        }
      }
    });

    c.addEventListener("pointerup", (ev) => {
      clearTimeout(longPressTimer);
      if (!pointers.has(ev.pointerId)) return;
      pointers.delete(ev.pointerId);
      const wasTap = ev.button === 0 && movedTotal < 8 && performance.now() - downAt < 250;
      if (dragNode) {
        if (this.engine) this.engine.setPinned(dragNode, false);
        if (wasTap) this.onPick(dragNode);
        dragNode = null;
        panning = false;
      } else if (panning && wasTap) {
        this.selected = null;
        this.onTransform({ selected: null });
      }
      panning = false;
      this.invalidate();
    });

    c.addEventListener("pointercancel", cancelGesture);

    c.addEventListener(
      "wheel",
      (ev) => {
        ev.preventDefault();
        const p = posOf(ev);
        const world = this.toWorld(p.x, p.y);
        const factor = Math.exp(-ev.deltaY * 0.0015);
        const base = this.fitScale();
        const s = Math.min(3 * base, Math.max(0.5 * base, this.scale * factor));
        this.scale = s;
        this.tx = p.x - world.x * s;
        this.ty = p.y - world.y * s;
        this.userTransformed = true;
        this.invalidate();
      },
      { passive: false }
    );

    c.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      if (ev.pointerType !== "touch") this.onBack();
    });
    c.addEventListener("pointerleave", () => this.clearHover());
  }

  selectNode(name) {
    this.selected = name;
    this.invalidate();
  }
  clearSelection() {
    this.selected = null;
    this.invalidate();
  }
  hoverNode(name) {
    this.hovered = name;
    this.invalidate();
  }
  clearHover() {
    this.hovered = null;
    this.invalidate();
  }
}

if (typeof window !== "undefined") {
  window.GraphEngine = GraphEngine;
  window.GraphCanvas = GraphCanvas;
  window.COLORS = COLORS;
  window.radiusOf = radiusOf;
}
if (typeof module !== "undefined" && module.exports) {
  module.exports = { GraphEngine, GraphCanvas, COLORS, radiusOf };
}
