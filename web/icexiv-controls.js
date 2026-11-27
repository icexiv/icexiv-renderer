import { TimelinePanel } from "./icexiv-timeline.js";

const stateEvents = ["loading", "ready", "timeupdate", "play", "pause", "ended", "emptied", "error"];
const speakerIcon = `<svg viewBox="0 0 20 20" aria-hidden="true"><path d="M3 7h3l4-3v12l-4-3H3z"/><g class="sound-waves" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M13 7a4 4 0 0 1 0 6M15.5 4.5a7.5 7.5 0 0 1 0 11"/></g><path class="mute-mark" d="m13 8 4 4m0-4-4 4" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/></svg>`;

function formatTime(seconds) {
  const ticks = Math.floor(Math.max(0, seconds) * 100);
  return `${Math.floor(ticks / 6000)}:${String(Math.floor(ticks / 100) % 60).padStart(2, "0")}.${String(ticks % 100).padStart(2, "0")}`;
}

/** Ranim-style playback & expandable multi-track timeline controls bound to <icexiv-viewport>. */
export class IcexivControls extends HTMLElement {
  static observedAttributes = ["for", "timeline-expanded"];

  constructor() {
    super();
    this.attachShadow({ mode: "open" }).innerHTML = `
      <style>
        :host {
          display: block; min-width: 0; container-type: inline-size;
          --surface: var(--paper, #ffffff);
          --text: var(--ink, #25332f);
          --muted: var(--dim, #60706b);
          --border: var(--line, #d7dfda);
          --hover: color-mix(in srgb, var(--text) 6%, var(--surface));
          --ctrl-accent: var(--accent, #17665d);
          --accent-soft: color-mix(in srgb, var(--ctrl-accent) 12%, var(--surface));
          --track: color-mix(in srgb, var(--text) 15%, var(--surface));
          --mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
        }
        * { box-sizing: border-box; }
        .controls {
          display: grid; grid-template-columns: 34px minmax(32px, 1fr) auto 28px 28px;
          align-items: center; gap: 10px; padding: 6px 12px;
          border-top: 1px solid var(--border); background: var(--surface);
        }
        button {
          display: grid; place-items: center; width: 34px; height: 34px; padding: 0;
          border: 1px solid transparent; border-radius: 4px;
          background: transparent; color: var(--muted); cursor: pointer;
        }
        button:hover:not(:disabled) { background: var(--hover); color: var(--text); }
        button:focus-visible { outline: 2px solid var(--ctrl-accent); outline-offset: 2px; }
        .playback { color: var(--ctrl-accent); background: var(--accent-soft); }
        button:disabled { opacity: .4; cursor: default; }
        button svg { width: 16px; height: 16px; fill: currentColor; }
        .timeline-toggle, .volume-toggle { width: 28px; height: 28px; }
        .mute-mark { display: none; }
        .timeline-toggle svg { fill: none; transition: transform 120ms; }
        :host([timeline-expanded]) .timeline-toggle svg { transform: rotate(180deg); }
        .timeline { overflow: hidden; --accent: var(--ctrl-accent); }
        .timeline[hidden] { display: none; }
        .pause-icon, button[data-playing] .play-icon { display: none; }
        button[data-playing] .pause-icon { display: block; }
        input {
          appearance: none; width: 100%; min-width: 0; height: 28px; margin: 0;
          background: transparent; cursor: pointer; --progress: 0%;
        }
        input::-webkit-slider-runnable-track {
          height: 4px; border-radius: 4px;
          background: linear-gradient(to right, var(--ctrl-accent) var(--progress), var(--track) var(--progress));
        }
        input::-moz-range-track {
          height: 4px; border-radius: 4px;
          background: linear-gradient(to right, var(--ctrl-accent) var(--progress), var(--track) var(--progress));
        }
        input::-webkit-slider-thumb {
          appearance: none; width: 10px; height: 10px; margin-top: -3px;
          border: 0; border-radius: 50%; background: var(--ctrl-accent);
        }
        input::-moz-range-thumb {
          width: 10px; height: 10px; border: 0; border-radius: 50%; background: var(--ctrl-accent);
        }
        input:disabled { opacity: .35; cursor: default; }
        output {
          color: var(--muted); font: 11px var(--mono);
          font-variant-numeric: tabular-nums; white-space: nowrap;
        }
        @container (max-width: 300px) {
          .controls { grid-template-columns: 34px minmax(32px, 1fr) 28px 28px; gap: 6px; }
          .time { grid-column: 2 / 5; grid-row: 2; text-align: right; }
        }
      </style>
      <div class="controls" part="controls">
        <button class="playback" type="button" aria-label="播放" title="播放" disabled>
          <svg class="play-icon" viewBox="0 0 20 20" aria-hidden="true"><path d="M5 3.8a1 1 0 0 1 1.5-.86l9.2 5.32a2 2 0 0 1 0 3.48L6.5 17.06A1 1 0 0 1 5 16.2Z"/></svg>
          <svg class="pause-icon" viewBox="0 0 20 20" aria-hidden="true"><rect x="5" y="3" width="3.5" height="14" rx="1"/><rect x="11.5" y="3" width="3.5" height="14" rx="1"/></svg>
        </button>
        <input type="range" min="0" max="0" step="any" value="0" aria-label="时间进度" disabled>
        <output class="time" aria-live="off">0:00.00 / 0:00.00</output>
        <button class="volume-toggle" type="button" aria-label="音效（即将支持）" title="音效（即将支持）" disabled>${speakerIcon}</button>
        <button class="timeline-toggle" type="button" aria-label="展开时间轴" title="展开时间轴" aria-expanded="false" aria-controls="timeline-panel" disabled>
          <svg viewBox="0 0 20 20" aria-hidden="true"><path d="m6 8 4 4 4-4" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </button>
      </div>
      <div class="timeline" id="timeline-panel" part="timeline" hidden></div>`;
    this._button = this.shadowRoot.querySelector(".playback");
    this._slider = this.shadowRoot.querySelector("input");
    this._output = this.shadowRoot.querySelector("output");
    this._timelineToggle = this.shadowRoot.querySelector(".timeline-toggle");
    this._timelinePanel = new TimelinePanel(
      this.shadowRoot.querySelector(".timeline"),
      (seconds) => this._target?.seek(seconds),
      (node, enabled) => this._target?.setTrackEnabled?.(node, enabled),
    );
    this._target = null;
    this._assignedTarget = undefined;
    this._observer = null;
    this._onStateChange = () => this._updateControls();
    this._button.addEventListener("click", () => {
      if (!this._target) return;
      if (this._target.paused) this._target.play();
      else this._target.pause();
    });
    this._slider.addEventListener("input", () => this._target?.seek(Number(this._slider.value)));
    this._timelineToggle.addEventListener("click", () => { this.timelineExpanded = !this.timelineExpanded; });
  }

  connectedCallback() {
    for (const property of ["target", "timelineExpanded"]) {
      if (Object.hasOwn(this, property)) {
        const value = this[property];
        delete this[property];
        this[property] = value;
      }
    }
    this._watchTarget();
    this._timelinePanel.connect();
    this._updateExpansion();
  }

  disconnectedCallback() {
    this._observer?.disconnect();
    this._observer = null;
    this._timelinePanel.disconnect();
    this._bind(null);
  }

  attributeChangedCallback(name) {
    if (name === "timeline-expanded") {
      this._updateExpansion();
      return;
    }
    this._assignedTarget = undefined;
    this._watchTarget();
  }

  get target() { return this._assignedTarget !== undefined ? this._assignedTarget : this._target; }
  set target(target) {
    this._assignedTarget = target;
    this._watchTarget();
  }

  get timelineExpanded() { return this.hasAttribute("timeline-expanded"); }
  set timelineExpanded(value) { this.toggleAttribute("timeline-expanded", Boolean(value)); }

  _updateExpansion() {
    const open = this.timelineExpanded;
    this._timelineToggle.setAttribute("aria-expanded", String(open));
    this._timelineToggle.setAttribute("aria-label", open ? "收起时间轴" : "展开时间轴");
    this._timelineToggle.title = open ? "收起时间轴" : "展开时间轴";
    this._timelinePanel.setVisible(open);
  }

  _watchTarget() {
    this._observer?.disconnect();
    this._observer = null;
    if (!this.isConnected) return;
    if (this._assignedTarget !== undefined) {
      this._bind(this._assignedTarget);
      return;
    }
    const id = this.getAttribute("for");
    if (!id) {
      this._bind(null);
      return;
    }
    const root = this.getRootNode();
    const resolve = () => this._bind(root.getElementById(id));
    resolve();
    this._observer = new MutationObserver(resolve);
    this._observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ["id"] });
  }

  _bind(target) {
    if (target !== this._target) {
      if (this._target) for (const ev of stateEvents) this._target.removeEventListener(ev, this._onStateChange);
      this._target = target;
      if (this._target) for (const ev of stateEvents) this._target.addEventListener(ev, this._onStateChange);
    }
    this._updateControls();
  }

  _updateControls() {
    const t = this._target;
    const duration = t?.duration ?? 0;
    const current = t?.currentTime ?? 0;
    const ready = Boolean(t && duration > 0);
    const playing = Boolean(ready && !t.paused);
    this._button.disabled = !ready;
    this._button.toggleAttribute("data-playing", playing);
    this._button.setAttribute("aria-label", playing ? "暂停" : "播放");
    this._button.title = playing ? "暂停" : "播放";
    this._slider.disabled = !ready;
    this._slider.max = String(duration);
    this._slider.value = String(current);
    this._slider.style.setProperty("--progress", `${duration > 0 ? (current / duration) * 100 : 0}%`);
    this._output.textContent = `${formatTime(current)} / ${formatTime(duration)}`;
    const tl = t?.timeline ?? null;
    this._timelineToggle.disabled = !tl?.roots?.length;
    this._timelinePanel.setTimeline(tl);
    this._timelinePanel.setTime(current);
  }
}

if (!customElements.get("icexiv-controls")) customElements.define("icexiv-controls", IcexivControls);
