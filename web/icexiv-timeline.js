// Ranim-inspired multi-track timeline panel for FFXIV action/VFX/prop/audio timelines.
const themeCss = `
:host {
  --surface: var(--paper, #ffffff);
  --subtle: var(--rail, #f4f6f5);
  --text: var(--ink, #25332f);
  --muted: var(--dim, #60706b);
  --border: var(--line, #d7dfda);
  --line: var(--line, #e6ece8);
  --hover: color-mix(in srgb, var(--text) 6%, var(--surface));
  --accent: var(--accent, #17665d);
  --accent-soft: color-mix(in srgb, var(--accent) 12%, var(--surface));
  --radius: 6px;
  --mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
}
`;

function element(tag, className, text) {
  const result = document.createElement(tag);
  result.className = className;
  if (text !== undefined) result.textContent = text;
  return result;
}

function seconds(value) { return `${Number(value.toFixed(3))}s`; }

function chainRange(chain, unitsPerSecond) {
  let start = 0;
  let duration = 0;
  for (const link of chain || []) {
    start += (link.actor_start || 0) + (link.track_start || 0) + (link.start || 0);
    duration = link.duration || duration;
  }
  const s = Math.max(0, start) / unitsPerSecond;
  const e = Math.max(s, (Math.max(0, start) + Math.max(0, duration)) / unitsPerSecond);
  return [s, e];
}

function shortLeaf(path) {
  const leaf = String(path || "").split("/").at(-1) || path;
  return leaf.replace(/\.(?:avfx|scd|mdl|pap|tmb)$/i, "");
}

/**
 * Project an FFXIV action bundle + preview config into a TimelinePanel tree:
 * `{ duration, loopRange, roots }`.
 */
export function projectFfxivTimeline(bundle, preview = {}) {
  const units = preview.units_per_second || 30;
  const enabledSet = new Set((preview.enabled_tracks || []).map(([tmb, tr]) => `${tmb}#${tr}`));
  const roots = [];
  let maxEnd = 0;

  const isTrackEnabled = (chain) => {
    for (const link of chain || []) {
      if (link.conditions && link.conditions.length > 0) {
        if (!enabledSet.has(`${link.tmb}#${link.track}`)) return false;
      }
    }
    return true;
  };

  // 1. Skeleton animation track (C010 / C009)
  const animChildren = (bundle.clips || []).map((clip, i) => {
    const [s, e] = chainRange(clip.chain, units);
    maxEnd = Math.max(maxEnd, e);
    return {
      name: clip.name,
      label: clip.name,
      kind: "anim",
      range: [s, e],
      contentDuration: Math.max(0, e - s),
      enabled: isTrackEnabled(clip.chain),
      toggleable: false,
      meta: { type: "clip", occurrence: i, tmb: clip.chain?.[0]?.tmb },
      children: [],
    };
  });
  if (animChildren.length) {
    const end = Math.max(...animChildren.map((c) => c.range[1]));
    roots.push({
      name: "骨骼动画 (C010)",
      label: "骨骼动画",
      kind: "children",
      range: [0, end],
      contentDuration: end,
      enabled: true,
      toggleable: false,
      children: animChildren,
    });
  }

  // 2. AVFX effects track (C012)
  const vfxChildren = (bundle.effects || []).map((fx, i) => {
    const [s, e] = chainRange(fx.chain, units);
    maxEnd = Math.max(maxEnd, e);
    const enabled = isTrackEnabled(fx.chain);
    return {
      name: fx.avfx,
      label: shortLeaf(fx.avfx),
      kind: "vfx",
      range: [s, e],
      contentDuration: Math.max(0, e - s),
      enabled,
      toggleable: enabled,
      meta: { type: "vfx", occurrence: i, targets: fx.targets },
      children: [],
    };
  });
  if (vfxChildren.length) {
    const end = Math.max(...vfxChildren.map((c) => c.range[1]));
    roots.push({
      name: "粒子特效 (C012)",
      label: "粒子特效",
      kind: "children",
      range: [0, end],
      contentDuration: end,
      enabled: true,
      toggleable: false,
      children: vfxChildren,
    });
  }

  // 3. 3D props track (C198)
  const propChildren = (bundle.props || []).map((prop, i) => {
    const [s, e] = chainRange(prop.chain, units);
    maxEnd = Math.max(maxEnd, e);
    const label = `w${String(prop.modelId).padStart(4, "0")}b${String(prop.bodyId).padStart(4, "0")} (${prop.bone})`;
    return {
      name: `${label} · v${prop.variantId}`,
      label,
      kind: "prop",
      range: [s, e],
      contentDuration: Math.max(0, e - s),
      enabled: true,
      toggleable: true,
      meta: { type: "prop", propId: `c198-${i}`, bone: prop.bone, modelMain: prop.modelMain },
      children: [],
    };
  });
  if (propChildren.length) {
    const end = Math.max(...propChildren.map((c) => c.range[1]));
    roots.push({
      name: "3D 道具 (C198)",
      label: "3D 道具",
      kind: "children",
      range: [0, end],
      contentDuration: end,
      enabled: true,
      toggleable: false,
      children: propChildren,
    });
  }

  // 4. Sound markers track (C063)
  const soundChildren = (bundle.sounds || []).map((snd, i) => {
    const [s, e] = chainRange(snd.chain, units);
    const end = Math.max(e, s + 0.12);
    maxEnd = Math.max(maxEnd, e);
    return {
      name: snd.scd,
      label: shortLeaf(snd.scd),
      kind: "audio",
      range: [s, end],
      contentDuration: Math.max(0, end - s),
      enabled: true,
      toggleable: false,
      meta: { type: "audio", occurrence: i },
      children: [],
    };
  });
  if (soundChildren.length) {
    const end = Math.max(...soundChildren.map((c) => c.range[1]));
    roots.push({
      name: "音效标记 (C063)",
      label: "音效标记",
      kind: "children",
      range: [0, end],
      contentDuration: end,
      enabled: true,
      toggleable: false,
      children: soundChildren,
    });
  }

  let loopRange = null;
  for (const ph of bundle.phases || []) {
    if (ph.kind === "loop" && ph.end > ph.start) {
      loopRange = [ph.start / units, ph.end / units];
      maxEnd = Math.max(maxEnd, ph.end / units);
    }
  }
  return { duration: Math.max(0.1, maxEnd), loopRange, roots };
}

export function projectTimeline(data) {
  function project(info, path, start, end, enabled) {
    const node = { ...info, id: path.join("."), start, end, enabled: enabled && info.enabled };
    node.label = info.label || info.name;
    node.serial = info.kind === "children" && info.children.every((child, i, children) =>
      i === 0 || children[i - 1].range[1] <= child.range[0]);
    const scale = info.contentDuration > 0 ? (end - start) / info.contentDuration : 0;
    node.children = (info.children || []).map((child, i) => project(child, [...path, i],
      start + child.range[0] * scale, start + child.range[1] * scale, node.enabled));
    return node;
  }
  return (data.roots || []).map((info, i) => project(info, [i], info.range[0], info.range[1], true));
}

function expandable(node) { return !node.serial && node.children.length > 0; }

export function defaultExpanded(roots) {
  const expanded = new Set();
  function visit(nodes, depth) {
    for (const node of nodes) {
      if (expandable(node) && depth < 2) expanded.add(node.id);
      visit(node.children, depth + Number(expandable(node)));
    }
  }
  visit(roots, 0);
  return expanded;
}

export function buildTimelineTracks(roots, expanded) {
  const rows = [];
  function visit(node, depth) {
    const clips = [], bands = [], children = [];
    function serialContent(container) {
      bands.push(container);
      for (const child of container.children) {
        if (child.serial) serialContent(child);
        else {
          clips.push(child);
          if (expandable(child) && expanded.has(child.id)) children.push(...child.children);
        }
      }
    }
    if (node.serial) serialContent(node);
    else {
      clips.push(node);
      if (expanded.has(node.id)) children.push(...node.children);
    }
    rows.push({ node, depth, clips, bands });
    for (const child of children) visit(child, depth + 1);
  }
  for (const root of roots) visit(root, 0);
  return rows;
}

/** Expandable multi-track timeline panel (aligned with ranim-timeline.js). */
export class TimelinePanel {
  constructor(host, seek, onToggleTrack) {
    this.host = host;
    this._seek = seek;
    this._onToggleTrack = onToggleTrack;
    this._data = null;
    this._roots = null;
    this._nodes = new Map();
    this._expanded = new Set();
    this._disabledTracks = new Set();
    this._selected = null;
    this._time = 0;
    this._zoom = 1;
    this._visible = false;
    this._scrub = null;
    this._root = host.attachShadow({ mode: "open" });
    this._root.innerHTML = `
      <style>
        ${themeCss}
        :host {
          display: block; min-width: 0; background: var(--surface);
          font-size: 12px; line-height: 1.4;
          --clip-anim: #397fae; --clip-vfx: #8b5cf6;
          --clip-prop: #9a6d39; --clip-audio: #327f61;
        }
        * { box-sizing: border-box; }
        button { font: inherit; cursor: pointer; }
        button:disabled { opacity: .35; cursor: default; }
        button:focus-visible, [tabindex]:focus-visible { outline: 2px solid var(--accent); outline-offset: -2px; }
        .toolbar { display: flex; align-items: center; justify-content: space-between; gap: 10px; padding: 6px 12px; border-top: 1px solid var(--border); }
        .title { font-size: 12px; font-weight: 600; color: var(--muted); }
        .zoom { display: flex; align-items: center; gap: 5px; }
        .zoom button { height: 24px; min-width: 25px; border: 1px solid var(--border); border-radius: 4px; background: var(--surface); color: var(--muted); }
        .zoom button:hover:not(:disabled) { background: var(--hover); color: var(--text); }
        .zoom-level { min-width: 38px; text-align: center; font: 10px var(--mono); color: var(--muted); }
        .tracks { overflow: auto; max-height: 280px; overscroll-behavior: contain; scrollbar-color: var(--border) var(--subtle); scrollbar-width: thin; }
        .sheet { position: relative; min-width: 100%; width: calc(var(--label-width) + var(--chart-width)); background: var(--surface); }
        .ruler, .row { display: grid; grid-template-columns: var(--label-width) var(--chart-width); }
        .ruler { position: sticky; top: 0; z-index: 6; height: 28px; }
        .ruler-label, .label { position: sticky; left: 0; z-index: 5; background: var(--subtle); border-right: 1px solid var(--border); }
        .ruler-label { display: flex; align-items: center; padding: 0 12px; color: var(--muted); font-size: 11px; }
        .ruler-grid { position: relative; background: var(--subtle); cursor: ew-resize; overflow: hidden; touch-action: none; }
        .tick { position: absolute; top: 0; bottom: 0; border-left: 1px solid var(--line); }
        .tick span { position: absolute; top: 5px; left: 5px; color: var(--muted); font: 10px var(--mono); white-space: nowrap; }
        .tick[data-end] span { left: auto; right: 5px; }
        .ruler-cursor { position: absolute; top: 0; bottom: 0; width: 2px; background: var(--accent); pointer-events: none; }
        .ruler-cursor::before { content: ''; position: absolute; top: 0; left: -4px; border: 5px solid transparent; border-top-color: var(--accent); }
        .row { height: 32px; border-bottom: 1px solid var(--line); }
        .label { display: flex; align-items: center; padding-left: calc(7px + var(--depth) * 13px); gap: 3px; }
        .tree-toggle { flex: 0 0 16px; width: 16px; height: 20px; padding: 0; border: 0; color: var(--muted); background: transparent; font-size: 10px; }
        .tree-spacer { flex: 0 0 16px; width: 16px; text-align: center; color: var(--muted); font-size: 9px; }
        .track-check { flex: 0 0 auto; margin: 0 2px 0 0; accent-color: var(--accent); cursor: pointer; }
        .node-label { flex: 1; min-width: 0; display: flex; align-items: center; gap: 5px; height: 100%; padding: 0 8px 0 0; border: 0; background: transparent; text-align: left; color: var(--text); }
        .node-label span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 12px; }
        .node-label small { margin-left: auto; color: var(--muted); font: 10px var(--mono); }
        .label[data-selected] { background: var(--accent-soft); }
        .time-grid { position: relative; overflow: hidden;
          background-image: linear-gradient(to right, var(--line) 1px, transparent 1px); background-size: var(--grid-step) 100%; }
        .serial-band { position: absolute; top: 2px; bottom: 2px; border-bottom: 1px solid color-mix(in srgb, var(--clip-prop) 25%, transparent); background: color-mix(in srgb, var(--clip-prop) 3%, transparent); pointer-events: none; }
        .clip { position: absolute; top: 6px; height: 20px; min-width: 3px; padding: 0 5px; border: 1px solid color-mix(in srgb, var(--clip-color) 32%, var(--surface));
          border-radius: 3px; background: color-mix(in srgb, var(--clip-color) 12%, var(--surface)); color: color-mix(in srgb, var(--clip-color) 70%, var(--text));
          font: 11px var(--mono); text-align: left; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
        .clip[data-expandable] { padding-left: 22px; }
        .clip:hover { background: color-mix(in srgb, var(--clip-color) 20%, var(--surface)); }
        .clip[data-selected] { border-color: var(--accent); box-shadow: inset 0 0 0 1px var(--accent); }
        [data-disabled] { opacity: .4; }
        .clip[data-disabled] { border-style: dashed; }
        .clip-toggle { position: absolute; top: 7px; height: 18px; width: 18px; padding: 0; border: 0; border-right: 1px solid var(--line); background: transparent; color: var(--muted); font-size: 10px; }
        .cursor { position: absolute; top: 28px; bottom: 0; width: 1.5px; background: var(--accent); pointer-events: none; z-index: 4; }
        .inspector { display: flex; flex-wrap: wrap; justify-content: space-between; gap: 4px 12px; padding: 8px 12px; border-top: 1px solid var(--border); min-height: 32px; background: var(--subtle); }
        .selection-name { min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; color: var(--muted); font-size: 11px; }
        .selection-range { color: var(--muted); font: 10px var(--mono); white-space: nowrap; }
        .empty { margin: 0; padding: 12px; color: var(--muted); font-size: 12px; }
        [hidden] { display: none !important; }
      </style>
      <div class="toolbar">
        <span class="title">时间轴</span>
        <div class="zoom">
          <button class="zoom-out" type="button" aria-label="缩小时间轴" title="缩小">−</button>
          <span class="zoom-level">100%</span>
          <button class="zoom-in" type="button" aria-label="放大时间轴" title="放大">+</button>
          <button class="fit" type="button" aria-label="适应全长" title="适应全长">适配</button>
        </div>
      </div>
      <p class="empty">加载动画或时间轴后即可查看轨道。</p>
      <div class="tracks" tabindex="0" role="region" aria-label="动作时间轴" hidden>
        <div class="sheet"></div>
      </div>
      <div class="inspector">
        <span class="selection-name">点击片段查看资源与时间区间</span>
        <span class="selection-range"></span>
      </div>`;
    this._scroller = this._root.querySelector(".tracks");
    this._sheet = this._root.querySelector(".sheet");
    this._resize = new ResizeObserver(() => { if (this._visible) this._layout(); });
    this._root.querySelector(".zoom-out").addEventListener("click", () => this._zoomBy(0.5));
    this._root.querySelector(".zoom-in").addEventListener("click", () => this._zoomBy(2));
    this._root.querySelector(".fit").addEventListener("click", () => {
      this._zoom = 1;
      this._layout();
      this._scroller.scrollLeft = 0;
    });
    this._sheet.addEventListener("pointerdown", (event) => this._startScrub(event));
    this._scroller.addEventListener("pointermove", (event) => {
      if (this._scrub?.id === event.pointerId) this._seekPointer(event.clientX);
    });
    for (const name of ["pointerup", "pointercancel", "lostpointercapture"]) {
      this._scroller.addEventListener(name, () => this._stopScrub());
    }
    this._scroller.addEventListener("wheel", (event) => {
      if (!(event.ctrlKey || event.metaKey) || !this._data) return;
      event.preventDefault();
      const rect = this._scroller.getBoundingClientRect();
      const anchor = Math.max(0, Math.min(1, (event.clientX - rect.left - this._labelWidth) / (rect.width - this._labelWidth)));
      this._zoomBy(Math.exp(-event.deltaY * 0.005), anchor);
    }, { passive: false });
  }

  connect() { this._resize.observe(this.host); }
  disconnect() { this._resize.disconnect(); this._stopScrub(); }

  setVisible(visible) {
    this._visible = visible;
    this.host.hidden = !visible;
    if (visible) this._render();
    else this._stopScrub();
  }

  setTimeline(data) {
    if (data === this._data) return;
    this._stopScrub();
    this._data = data;
    this._roots = null;
    this._nodes.clear();
    this._disabledTracks.clear();
    this._selected = null;
    this._zoom = 1;
    this._scroller.scrollLeft = this._scroller.scrollTop = 0;
    this._selection(null);
    if (this._visible) this._render();
  }

  setTime(time) {
    this._time = time;
    if (!this._visible || !this._data) return;
    const fraction = Math.max(0, Math.min(1, time / this._scaleDuration()));
    this._cursor?.style.setProperty("left", `calc(var(--label-width) + var(--chart-width) * ${fraction})`);
    this._rulerCursor?.style.setProperty("left", `${fraction * 100}%`);
    this._ruler?.setAttribute("aria-valuenow", String(time));
    this._ruler?.setAttribute("aria-valuetext", seconds(time));
  }

  _scaleDuration() { return Math.max(this._data?.duration ?? 0, 0.1); }

  _isNodeEnabled(node) { return node.enabled && !this._disabledTracks.has(node.id); }

  _render() {
    const available = (this._data?.roots?.length ?? 0) > 0;
    this._root.querySelector(".empty").hidden = available;
    this._scroller.hidden = !available;
    for (const button of this._root.querySelectorAll(".zoom button")) button.disabled = !available;
    if (!available) {
      this._sheet.replaceChildren();
      this._cursor = this._ruler = this._rulerCursor = null;
      return;
    }
    if (!this._roots) {
      this._roots = projectTimeline(this._data);
      const remember = (node) => { this._nodes.set(node.id, node); node.children.forEach(remember); };
      this._roots.forEach(remember);
      this._expanded = defaultExpanded(this._roots);
    }
    const ruler = element("div", "ruler");
    ruler.append(element("div", "ruler-label", "轨道"));
    this._ruler = element("div", "ruler-grid");
    this._ruler.tabIndex = 0;
    this._ruler.setAttribute("role", "slider");
    this._ruler.setAttribute("aria-label", "时间轴游标");
    this._ruler.setAttribute("aria-valuemin", "0");
    this._ruler.setAttribute("aria-valuemax", String(this._data.duration));
    this._ruler.addEventListener("keydown", (event) => {
      const delta = this._data.duration / 100;
      const time = { ArrowLeft: this._time - delta, ArrowRight: this._time + delta, Home: 0, End: this._data.duration }[event.key];
      if (time !== undefined) { event.preventDefault(); this._seekTime(time); }
    });
    ruler.append(this._ruler);
    this._sheet.replaceChildren(ruler);
    for (const row of buildTimelineTracks(this._roots, this._expanded)) this._sheet.append(this._row(row));
    this._cursor = element("div", "cursor");
    this._sheet.append(this._cursor);
    this._selection(this._selected);
    this._layout();
  }

  _row({ node, depth, clips, bands }) {
    const row = element("div", "row");
    const label = element("div", "label");
    label.style.setProperty("--depth", depth);
    label.dataset.node = node.id;
    if (expandable(node)) label.append(this._toggle(node, "tree-toggle"));
    else label.append(element("span", "tree-spacer", node.serial ? "≡" : "·"));
    const toggleTargets = node.toggleable ? [node] : clips.filter((c) => c.toggleable);
    if (toggleTargets.length && this._onToggleTrack) {
      const check = document.createElement("input");
      check.type = "checkbox";
      check.className = "track-check";
      check.checked = toggleTargets.some((t) => this._isNodeEnabled(t));
      check.title = "启用/隐藏该轨道";
      check.addEventListener("change", () => {
        for (const t of toggleTargets) {
          if (check.checked) this._disabledTracks.delete(t.id);
          else this._disabledTracks.add(t.id);
          this._onToggleTrack(t, check.checked);
        }
        this._render();
      });
      label.append(check);
    }
    const name = element("button", "node-label");
    name.type = "button";
    name.dataset.node = node.id;
    name.dataset.select = "";
    name.title = node.name;
    name.append(element("span", "", node.label), element("small", "", seconds(node.end - node.start)));
    name.addEventListener("click", () => this._selection(node.id));
    name.toggleAttribute("data-disabled", !this._isNodeEnabled(node));
    label.append(name);

    const grid = element("div", "time-grid");
    for (const band of bands) {
      const line = element("div", "serial-band");
      this._place(line, band);
      grid.append(line);
    }
    for (const clip of clips) {
      const enabled = this._isNodeEnabled(clip);
      const button = element("button", "clip", clip.label);
      button.type = "button";
      button.dataset.node = clip.id;
      button.dataset.select = "";
      button.title = `${clip.name}\n${seconds(clip.start)} — ${seconds(clip.end)}${enabled ? "" : " · 已禁用"}`;
      button.toggleAttribute("data-expandable", expandable(clip));
      button.toggleAttribute("data-disabled", !enabled);
      const color = clip.kind === "vfx" ? "var(--clip-vfx)"
        : clip.kind === "prop" ? "var(--clip-prop)"
        : clip.kind === "audio" ? "var(--clip-audio)"
        : clip.kind === "children" ? "var(--clip-prop)"
        : "var(--clip-anim)";
      button.style.setProperty("--clip-color", color);
      this._place(button, clip);
      button.addEventListener("click", () => this._selection(clip.id));
      grid.append(button);
      if (expandable(clip)) {
        const toggle = this._toggle(clip, "clip-toggle");
        toggle.style.left = `${clip.start / this._scaleDuration() * 100}%`;
        grid.append(toggle);
      }
    }
    row.append(label, grid);
    return row;
  }

  _toggle(node, className) {
    const open = this._expanded.has(node.id);
    const button = element("button", className, open ? "▾" : "▸");
    button.type = "button";
    button.dataset.toggle = node.id;
    button.setAttribute("aria-label", `${open ? "折叠" : "展开"} ${node.label}`);
    button.setAttribute("aria-expanded", String(open));
    button.addEventListener("click", () => {
      if (open) this._expanded.delete(node.id);
      else this._expanded.add(node.id);
      this._render();
      this._root.querySelector(`[data-toggle="${node.id}"]`)?.focus({ preventScroll: true });
    });
    return button;
  }

  _place(el, node) {
    el.style.left = `${node.start / this._scaleDuration() * 100}%`;
    el.style.width = `${Math.max(0, node.end - node.start) / this._scaleDuration() * 100}%`;
  }

  _selection(id) {
    this._selected = id;
    const node = this._nodes.get(id);
    for (const item of this._sheet.querySelectorAll("[data-node]")) {
      const selected = item.dataset.node === id;
      item.toggleAttribute("data-selected", selected);
      if (item.hasAttribute("data-select")) item.setAttribute("aria-pressed", String(selected));
    }
    const name = this._root.querySelector(".selection-name");
    name.textContent = node ? node.name : "点击片段查看资源与时间区间";
    name.title = node?.name ?? "";
    const enabled = node ? this._isNodeEnabled(node) : true;
    this._root.querySelector(".selection-range").textContent = node
      ? `${seconds(node.start)} — ${seconds(node.end)} · ${seconds(node.end - node.start)}${enabled ? "" : " · 已禁用"}` : "";
  }

  _layout() {
    if (!this._data || !this._ruler) return;
    this._labelWidth = Math.min(200, Math.max(120, this._scroller.clientWidth * 0.28));
    this._chartWidth = Math.max(180, this._scroller.clientWidth - this._labelWidth) * this._zoom;
    this._sheet.style.setProperty("--label-width", `${this._labelWidth}px`);
    this._sheet.style.setProperty("--chart-width", `${this._chartWidth}px`);
    const rough = this._scaleDuration() / Math.max(1, this._chartWidth / 85);
    const power = 10 ** Math.floor(Math.log10(rough));
    const step = [1, 2, 5, 10].map((v) => v * power).find((v) => v >= rough);
    this._sheet.style.setProperty("--grid-step", `${step / this._scaleDuration() * 100}%`);
    this._ruler.replaceChildren();
    const ticks = [];
    for (let time = 0; time < this._data.duration; time += step) {
      if (time === 0 || (this._data.duration - time) / this._scaleDuration() * this._chartWidth >= 55) ticks.push(time);
    }
    ticks.push(this._data.duration);
    for (const time of ticks) {
      const tick = element("div", "tick");
      tick.style.left = `${time / this._scaleDuration() * 100}%`;
      tick.toggleAttribute("data-end", time > 0 && time === this._data.duration);
      tick.append(element("span", "", seconds(time)));
      this._ruler.append(tick);
    }
    this._rulerCursor = element("div", "ruler-cursor");
    this._ruler.append(this._rulerCursor);
    this._root.querySelector(".zoom-level").textContent = `${Math.round(this._zoom * 100)}%`;
    this._root.querySelector(".zoom-out").disabled = this._zoom <= 1;
    this._root.querySelector(".zoom-in").disabled = this._zoom >= 32;
    this.setTime(this._time);
  }

  _zoomBy(factor, anchor = 0.5) {
    if (!this._data || !this._chartWidth) return;
    this._stopScrub();
    const width = this._scroller.clientWidth - this._labelWidth;
    const position = (this._scroller.scrollLeft + width * anchor) / this._chartWidth;
    this._zoom = Math.max(1, Math.min(32, this._zoom * factor));
    this._layout();
    this._scroller.scrollLeft = position * this._chartWidth - width * anchor;
  }

  _seekTime(time) { this._seek(Math.max(0, Math.min(this._data.duration, time))); }

  _startScrub(event) {
    const grid = event.target.closest(".ruler-grid");
    if (!grid || event.button !== 0) return;
    event.preventDefault();
    this._ruler.focus({ preventScroll: true });
    this._scrub = { id: event.pointerId, rect: grid.getBoundingClientRect() };
    this._scroller.setPointerCapture(event.pointerId);
    this._seekPointer(event.clientX);
  }

  _seekPointer(x) {
    const { left, width } = this._scrub.rect;
    if (width > 0) this._seekTime((x - left) / width * this._scaleDuration());
  }

  _stopScrub() {
    const id = this._scrub?.id;
    this._scrub = null;
    if (id !== undefined && this._scroller.hasPointerCapture(id)) this._scroller.releasePointerCapture(id);
  }
}
