import init, { EquipmentViewer } from "./icexiv_renderer.js";

/** Create in the document, await open(), load into its EquipmentViewer, then show(). */
export class EquipmentCanvas extends HTMLElement {
  #viewer; #observer; #raf; #epoch = 0; #drag;
  #yaw = 0.35; #pitch = 0.1; #zoom = 1.4;
  #dark=matchMedia("(prefers-color-scheme: dark)");
  constructor() {
    super();
    const root = this.attachShadow({mode: "open"});
    root.innerHTML = `<style>
      :host{display:block;margin-block:.75rem;max-width:64rem;color:var(--ink,#25332f);font:inherit}
      figure{margin:0;background:var(--rail,#eef1ef);border:1px solid var(--line,#d7dfda);border-radius:.6rem;overflow:hidden}
      canvas{display:block;width:100%;height:clamp(16rem,45vw,29rem);touch-action:none;cursor:grab}
      canvas:active{cursor:grabbing}canvas:focus-visible{outline:2px solid var(--accent,#17665d);outline-offset:-3px}
      figcaption{display:flex;flex-wrap:wrap;gap:.4rem 1rem;align-items:center;justify-content:space-between;padding:.45rem .8rem;border-top:1px solid var(--line,#d7dfda);font-size:.85rem;background:var(--paper,white)}
      nav{display:flex;align-items:center;gap:.2rem}button{font:inherit;color:inherit;background:none;border:0;padding:.4rem .55rem;border-radius:.25rem;cursor:pointer}
      button:hover{background:var(--rail,#eef1ef)}output{min-width:3em;text-align:center;font-variant-numeric:tabular-nums}
      </style><figure><canvas tabindex="0" aria-label="装备模型，方向键旋转，加减键缩放，Home 复位"></canvas>
      <figcaption><span>拖动旋转 · 滚轮缩放</span><nav aria-label="模型视角"><button data-step="-1" aria-label="缩小">−</button>
      <output>100%</output><button data-step="1" aria-label="放大">＋</button><button data-reset>复位</button></nav></figcaption></figure>`;
    const c = root.querySelector("canvas");
    c.onpointerdown = e => {
      if (!e.isPrimary || e.button !== 0) return;
      c.setPointerCapture(e.pointerId); this.#drag = [e.clientX,e.clientY];
    };
    c.onlostpointercapture = () => { this.#drag = null; };
    c.onpointermove = e => {
      if (!this.#drag || !c.hasPointerCapture(e.pointerId)) return;
      this.#orbit((this.#drag[0]-e.clientX)*0.01,(e.clientY-this.#drag[1])*0.01);
      this.#drag=[e.clientX,e.clientY];
    };
    c.addEventListener("wheel", e => {
      if (e.ctrlKey) return;
      e.preventDefault(); this.#scale(Math.exp(-Math.sign(e.deltaY)*0.09));
    }, {passive:false});
    c.onkeydown = e => {
      const move={ArrowLeft:[-.14,0],ArrowRight:[.14,0],ArrowUp:[0,.12],ArrowDown:[0,-.12]}[e.key];
      if(move) this.#orbit(...move);
      else if(e.key==="Home") this.#reset();
      else if(["+","=","-"].includes(e.key)) this.#scale(e.key==="-"?1/1.12:1.12);
      else return;
      e.preventDefault();
    };
    root.querySelector("nav").onclick=e=>{
      if(e.target.hasAttribute("data-reset")) this.#reset();
      else if(e.target.dataset.step) this.#scale(1.12 ** Number(e.target.dataset.step));
    };
  }
  #orbit(y,p){this.#yaw+=y;this.#pitch=Math.max(-1.35,Math.min(1.35,this.#pitch+p));this.frame();}
  #scale(f){this.#zoom=Math.min(1.8,Math.max(.65,this.#zoom*f));this.frame();}
  #reset(){this.#yaw=.35;this.#pitch=.1;this.#zoom=1.4;this.frame();}
  #resize(){
    const c=this.shadowRoot.querySelector("canvas"), box=c.getBoundingClientRect();
    const ratio=Math.min(devicePixelRatio||1,2.5,2560/Math.max(1,box.width,box.height));
    const w=Math.max(1,Math.round(box.width*ratio)),h=Math.max(1,Math.round(box.height*ratio));
    if(c.width!==w)c.width=w;
    if(c.height!==h)c.height=h;
  }
  async open(){
    this.dispose(); const epoch=this.#epoch;
    await init();
    this.#resize();
    const viewer=await EquipmentViewer.create(this.shadowRoot.querySelector("canvas"));
    if(epoch!==this.#epoch || !this.isConnected){viewer.free();throw Error("模型视图已关闭");}
    this.#viewer=viewer;
    this.#observer=new ResizeObserver(()=>this.frame());
    this.#observer.observe(this.shadowRoot.querySelector("canvas"));
    window.addEventListener("resize",this.frame);
    this.#dark.addEventListener("change",this.frame);
    return viewer;
  }
  frame=()=>{
    if(!this.#viewer || this.#raf) return;
    this.#raf=requestAnimationFrame(()=>{
      this.#raf=0;this.#resize();
      this.shadowRoot.querySelector("output").textContent=`${Math.round(this.#zoom/1.4*100)}%`;
      if(this.#loaded) {
        try {
          this.#viewer.setBackground(...(this.#dark.matches?[51,62,63]:[196,205,202]));
          this.#viewer.drawZoom(this.#yaw,this.#pitch,this.#zoom);
        }
        catch(error){this.dispatchEvent(new CustomEvent("rendererror",{detail:error}));}
      }
    });
  };
  #loaded=false;
  show(){this.#loaded=true;this.frame();}
  dispose(){
    this.#epoch++;this.#loaded=false;
    cancelAnimationFrame(this.#raf);this.#raf=0;
    window.removeEventListener("resize",this.frame);this.#observer?.disconnect();
    this.#dark.removeEventListener("change",this.frame);
    this.#viewer?.free();this.#viewer=undefined;
  }
  disconnectedCallback(){this.dispose();}
}
if (!customElements.get("icexiv-equipment-canvas")) customElements.define("icexiv-equipment-canvas",EquipmentCanvas);
