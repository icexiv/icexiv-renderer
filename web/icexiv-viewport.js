import { projectFfxivTimeline } from "./icexiv-timeline.js";

const STUDIO_RGB = [37, 44, 49];
const HOME = { yaw: 0.35, pitch: 0.1, zoom: 1, pan: [0, 0] };
const PRESETS = {
  full: { yaw: 0.35, pitch: 0.1, zoom: 1.0, pan: [0, 0] },
  upper: { yaw: 0.25, pitch: 0.08, zoom: 1.35, pan: [0, -0.22] },
  head: { yaw: 0.18, pitch: 0.04, zoom: 1.85, pan: [0, -0.42] },
};
const IDENTITY = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
const orbitDistance = (zoom, w, h) => 2.8 / zoom / Math.min(1, w / h);

/** Unified PBR + VFX + Actor WebGPU scene viewport. Pair with <icexiv-controls>. */
export class IcexivViewport extends HTMLElement {
  #worker; #observer; #visibility; #raf = 0; #epoch = 0; #drag = null;
  #onScreen = true; #canvasUsed = false; #loaded = false;
  #pending = new Map(); #seq = 0; #busy = false; #dirty = false;
  #yaw = HOME.yaw; #pitch = HOME.pitch; #zoom = HOME.zoom; #pan = [...HOME.pan];
  #paused = true; #lastTime; #currentTime = 0; #duration = 0; #loopRange = null;
  #timeline = null; #actionActive = false; #actorPlaying = false; #reported = false;
  #props = []; #disabledProps = new Set(); #pendingSeek = null;

  constructor() {
    super();
    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `<style>
      :host { display: block; min-width: 0; position: relative; background: rgb(${STUDIO_RGB.join(" ")}); }
      :host([hidden]) { display: none; }
      canvas { display: block; width: 100%; height: var(--viewport-height, clamp(16rem, 45vw, 29rem)); touch-action: none; cursor: grab; background: rgb(${STUDIO_RGB.join(" ")}); }
      canvas:active { cursor: grabbing; }
      canvas:focus-visible { outline: 2px solid var(--accent, #17665d); outline-offset: -3px; }
    </style><canvas tabindex="0" aria-label="3D 预览画布，拖动旋转，滚轮缩放，方向键微调，Home 复位"></canvas>`;
    this.#bindCanvas(root.querySelector("canvas"));
  }

  get paused() { return this.#paused; }
  get currentTime() { return this.#currentTime; }
  get duration() { return this.#duration; }
  get timeline() { return this.#timeline; }
  get zoom() { return this.#zoom; }

  #emit(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { bubbles: true, detail }));
  }

  #bindCanvas(c) {
    c.onpointerdown = (e) => {
      if (!e.isPrimary || e.button !== 0) return;
      c.setPointerCapture(e.pointerId);
      this.#drag = [e.clientX, e.clientY];
    };
    c.onlostpointercapture = () => { this.#drag = null; };
    c.onpointermove = (e) => {
      if (!this.#drag || !c.hasPointerCapture(e.pointerId)) return;
      this.#orbit((this.#drag[0] - e.clientX) * 0.01, (e.clientY - this.#drag[1]) * 0.01);
      this.#drag = [e.clientX, e.clientY];
    };
    c.addEventListener("wheel", (e) => {
      if (e.ctrlKey) return;
      e.preventDefault();
      this.#scale(Math.exp(-Math.sign(e.deltaY) * 0.09));
    }, { passive: false });
    c.onkeydown = (e) => {
      const move = { ArrowLeft: [-0.14, 0], ArrowRight: [0.14, 0], ArrowUp: [0, 0.12], ArrowDown: [0, -0.12] }[e.key];
      if (move) this.#orbit(...move);
      else if (e.key === "Home") this.resetCamera();
      else if (["+", "=", "-"].includes(e.key)) this.#scale(e.key === "-" ? 1 / 1.12 : 1.12);
      else return;
      e.preventDefault();
    };
  }

  #orbit(y, p) { this.#yaw += y; this.#pitch = Math.max(-1.35, Math.min(1.35, this.#pitch + p)); this.frame(); }
  #scale(f) { this.#zoom = Math.min(2.4, Math.max(0.5, this.#zoom * f)); this.frame(); }

  setZoom(zoom) { this.#zoom = Math.min(2.4, Math.max(0.5, Number(zoom) || 1)); this.frame(); }

  async setCameraPreset(name) {
    const p = PRESETS[name] || HOME;
    this.#yaw = p.yaw; this.#pitch = p.pitch; this.#zoom = p.zoom; this.#pan = [...p.pan];
    if (this.#worker) await this.#send("setPan", { x: p.pan[0], y: p.pan[1] });
    this.frame();
  }

  resetCamera() { return this.setCameraPreset("full"); }

  #resize() {
    const c = this.shadowRoot.querySelector("canvas"), box = c.getBoundingClientRect();
    const ratio = Math.min(devicePixelRatio || 1, 2.5, 2560 / Math.max(1, box.width, box.height));
    return [Math.max(1, Math.round(box.width * ratio)), Math.max(1, Math.round(box.height * ratio))];
  }

  #send(type, args = {}, transfer = []) {
    if (!this.#worker) return Promise.reject(Error("场景尚未打开"));
    return new Promise((resolve, reject) => {
      const id = ++this.#seq;
      this.#pending.set(id, { resolve, reject });
      this.#worker.postMessage({ ...args, type, id }, transfer);
    });
  }

  #progress(phase, done = 0, total = 0) {
    this.#emit("loadprogress", { phase, done, total });
  }

  async #entries(entries, epoch, phase, read) {
    if (entries.length > 1024) throw Error("Too many resources in bundle");
    const it = entries.values(), result = [];
    let active = true, total = 0;
    this.#progress(phase, 0, entries.length);
    const run = async () => {
      for (const [path, hash] of it) {
        if (!active || epoch !== this.#epoch) throw Error("场景已关闭");
        const bytes = await read(path, hash);
        if (!(bytes instanceof Uint8Array) || bytes.byteLength > (64 << 20) || (total += bytes.byteLength) > (128 << 20)) {
          throw Error(`Invalid or oversized resource at ${path}`);
        }
        result.push([path, bytes]);
        if (active && epoch === this.#epoch) this.#progress(phase, result.length, entries.length);
      }
    };
    try { await Promise.all(Array.from({ length: 4 }, run)); } finally { active = false; }
    if (epoch !== this.#epoch) throw Error("场景已关闭");
    return result;
  }

  /** Open a unified empty PBR + VFX WebGPU scene (no fake weapon required). */
  async openScene() {
    this.dispose();
    const epoch = this.#epoch;
    this.#emit("loading");
    this.#progress("engine");
    const canvas = this.shadowRoot.querySelector("canvas").transferControlToOffscreen();
    this.#canvasUsed = true;
    const worker = this.#worker = new Worker(new URL("./effects/effect-worker.js", import.meta.url), { type: "module" });
    worker.onmessage = ({ data: { id, value, error, phase } }) => {
      if (phase) { if (this.#pending.has(id)) this.#progress(phase); return; }
      const call = this.#pending.get(id); this.#pending.delete(id);
      if (error) call?.reject(Error(error)); else call?.resolve(value);
    };
    worker.onerror = (e) => {
      const error = Error(e.message || "渲染 Worker 加载失败");
      for (const call of this.#pending.values()) call.reject(error);
      this.#pending.clear();
      this.#emit("error", error);
    };
    await this.#send("openScene", { canvas }, [canvas]);
    if (epoch !== this.#epoch) return;
    this.#observer = new ResizeObserver(() => this.frame());
    this.#observer.observe(this.shadowRoot.querySelector("canvas"));
    window.addEventListener("resize", this.frame);
    this.#visibility = new IntersectionObserver(([e]) => { this.#onScreen = e.isIntersecting; this.#wake(); });
    this.#visibility.observe(this);
    document.addEventListener("visibilitychange", this.#wake);
    this.#loaded = true;
    this.frame();
  }

  async setActorBundle(manifest, read) {
    if (!this.#worker) await this.openScene();
    const { files, customize, equipment, sklb, pap, record } = manifest;
    const entries = await this.#entries(Object.entries(files ?? {}), this.#epoch, "actor-files", read);
    const byPath = new Map(entries);
    await this.#send("setActor", {
      entries: entries.filter(([p]) => p !== pap),
      customize: Uint8Array.from(customize),
      sklb: byPath.get(sklb),
      pap: byPath.get(pap),
      equipment: equipment ?? [],
      record: record ?? 0,
      bone: "n_buki_r",
      loop: true,
      grip: IDENTITY,
    });
    this.#actorPlaying = true;
    this.#wake();
  }

  async playActionBundle(bundle, preview, read) {
    if (!this.#worker) throw Error("场景尚未打开");
    const epoch = this.#epoch;
    await this.#send("clearProps");
    this.#props = [];
    this.#disabledProps.clear();

    const papPaths = new Set(Object.values(bundle.paps ?? {}));
    const isMotion = (p) => p === bundle.sklb || papPaths.has(p);
    const entries = await this.#entries(Object.entries(bundle.files ?? {}), epoch, "action-files", read);
    const byPath = new Map(entries);

    // Mount any C198 3D props onto their target actor bones.
    const units = preview?.units_per_second || 30;
    const seenProps = new Set();
    for (const [i, prop] of (bundle.props || []).entries()) {
      const id = `c198-${i}`;
      let start = 0, dur = 0;
      for (const link of prop.chain || []) {
        start += (link.actor_start || 0) + (link.track_start || 0) + (link.start || 0);
        dur = link.duration || dur;
      }
      const range = [Math.max(0, start) / units, Math.max(0, start + dur) / units];
      if (!seenProps.has(id)) {
        seenProps.add(id);
        await this.#send("addProp", {
          propId: id, main: prop.modelMain, bone: prop.bone || "n_buki_r",
          grip: IDENTITY, entries: entries.filter(([p]) => !isMotion(p)),
        });
      }
      this.#props.push({ id, start: range[0], end: range[1], visible: true });
    }

    const motion = bundle.clips?.length ? {
      target: "actor", gap: "rest", bone: "n_buki_r", grip: IDENTITY,
      sklb: byPath.get(bundle.sklb),
      paps: Object.fromEntries(Object.entries(bundle.paps).map(([n, p]) => [n, byPath.get(p)])),
    } : undefined;
    const warnings = await this.#send("play-action", {
      events: { effects: bundle.effects ?? [], clips: bundle.clips ?? [] },
      preview: preview ?? { units_per_second: 30, bindings: [], enabled_tracks: [] },
      entries: entries.filter(([p]) => !isMotion(p)),
      motion,
    });
    if (epoch !== this.#epoch) return warnings;

    const tl = projectFfxivTimeline(bundle, preview);
    this.#timeline = tl;
    this.#duration = tl.duration;
    this.#loopRange = tl.loopRange;
    this.#currentTime = 0;
    this.#actionActive = true;
    this.#paused = false;
    this.#reported = false;
    await this.#syncPropsAt(0);
    this.#emit("ready");
    this.#emit("play");
    this.#emit("timeupdate", { currentTime: 0, duration: this.#duration });
    this.#wake();
    return warnings;
  }

  play() {
    if (!this.#actionActive || !this.#paused) return;
    this.#paused = false;
    this.#lastTime = undefined;
    this.#emit("play");
    this.#wake();
  }

  pause() {
    if (this.#paused) return;
    this.#paused = true;
    this.#lastTime = undefined;
    this.#emit("pause");
  }

  seek(seconds) {
    if (!this.#actionActive) return;
    const clamped = Math.max(0, Math.min(this.#duration, Number(seconds) || 0));
    this.#currentTime = clamped;
    this.#pendingSeek = clamped;
    this.#emit("timeupdate", { currentTime: clamped, duration: this.#duration });
    this.frame();
  }

  async setTrackEnabled(node, enabled) {
    const meta = node?.meta;
    if (!meta || !this.#worker) return;
    if (meta.type === "vfx") {
      await this.#send("setActionOccurrenceEnabled", { occurrence: meta.occurrence, enabled });
    } else if (meta.type === "prop") {
      if (enabled) this.#disabledProps.delete(meta.propId);
      else this.#disabledProps.add(meta.propId);
      await this.#syncPropsAt(this.#currentTime);
    }
    this.frame();
  }

  async #syncPropsAt(t) {
    for (const p of this.#props) {
      const want = !this.#disabledProps.has(p.id) && t >= p.start && t <= p.end;
      if (want !== p.visible) {
        p.visible = want;
        await this.#send("setPropVisible", { propId: p.id, visible: want });
      }
    }
  }

  #wake = () => { this.#lastTime = undefined; this.frame(); };

  frame = () => {
    if (this.#busy) { this.#dirty = true; return; }
    if (!this.#worker || !this.#loaded || this.#raf) return;
    this.#raf = requestAnimationFrame(async (now) => {
      const epoch = this.#epoch;
      this.#raf = 0;
      if (this.hidden || document.hidden || !this.#onScreen) { this.#lastTime = undefined; return; }
      const [w, h] = this.#resize();
      this.#busy = true;
      this.#dirty = false;
      try {
        if (this.#pendingSeek !== null) {
          const t = this.#pendingSeek;
          this.#pendingSeek = null;
          await this.#send("seekAction", { seconds: t });
          await this.#syncPropsAt(t);
          this.#lastTime = now;
        }
        const dt = this.#paused || this.#lastTime === undefined ? 0 : (now - this.#lastTime) / 1000;
        const result = await this.#send("frame", {
          w, h, yaw: this.#yaw, pitch: this.#pitch,
          distance: orbitDistance(this.#zoom, w, h),
          dt,
          enabled: this.#actionActive,
          report: this.#actionActive && !this.#reported,
        });
        if (epoch !== this.#epoch) return;
        if (this.#actionActive && result?.age !== undefined) {
          let age = result.age;
          // Multi-phase seamless loop: play start once, then loop [loopRange[0], loopRange[1]].
          const loopEnd = this.#loopRange ? this.#loopRange[1] : this.#duration;
          const loopStart = this.#loopRange ? this.#loopRange[0] : 0;
          if (!this.#paused && loopEnd > 0 && age >= loopEnd) {
            await this.#send("seekAction", { seconds: loopStart });
            age = loopStart;
          }
          this.#currentTime = Math.min(age, this.#duration);
          await this.#syncPropsAt(this.#currentTime);
          this.#emit("timeupdate", { currentTime: this.#currentTime, duration: this.#duration });
        }
        const warnings = Array.isArray(result) ? result : result?.warnings;
        if (warnings) {
          this.#reported = true;
          if (warnings.length) this.#emit("renderwarning", warnings);
        }
        this.#lastTime = this.#paused ? undefined : now;
      } catch (error) {
        if (epoch === this.#epoch) this.#emit("error", error);
      } finally {
        if (epoch === this.#epoch) {
          this.#busy = false;
          if (this.#dirty || this.#pendingSeek !== null || (!this.#paused && (this.#actionActive || this.#actorPlaying))) {
            this.frame();
          }
        }
      }
    });
  };

  dispose() {
    this.#epoch++;
    this.#loaded = false;
    this.#actionActive = false;
    this.#actorPlaying = false;
    this.#paused = true;
    this.#timeline = null;
    this.#props = [];
    this.#disabledProps.clear();
    this.#pendingSeek = null;
    cancelAnimationFrame(this.#raf);
    this.#raf = 0;
    window.removeEventListener("resize", this.frame);
    this.#observer?.disconnect();
    this.#visibility?.disconnect();
    document.removeEventListener("visibilitychange", this.#wake);
    this.#worker?.terminate();
    this.#worker = undefined;
    if (this.#canvasUsed) {
      const prev = this.shadowRoot.querySelector("canvas"), next = prev.cloneNode(false);
      prev.replaceWith(next);
      this.#bindCanvas(next);
      this.#canvasUsed = false;
      this.#drag = null;
    }
    for (const call of this.#pending.values()) call.reject(Error("场景已关闭"));
    this.#pending.clear();
    this.#busy = this.#dirty = this.#reported = false;
    this.#lastTime = undefined;
    this.#onScreen = true;
  }

  disconnectedCallback() { this.dispose(); }
}

if (!customElements.get("icexiv-viewport")) customElements.define("icexiv-viewport", IcexivViewport);
