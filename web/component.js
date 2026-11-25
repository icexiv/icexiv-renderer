import init, { EquipmentViewer } from "./icexiv_renderer.js";

// Companion's studio clear after its HDR compose pass, expressed in display sRGB.
const STUDIO_RGB=[37,44,49];
const HOME={yaw:.35,pitch:.1,zoom:1};
const orbitDistance=(zoom,w,h)=>2.8/zoom/Math.min(1,w/h);
/** Create in the document, await open(), load into its EquipmentViewer, then show(). */
export class EquipmentCanvas extends HTMLElement {
  #viewer; #observer; #raf; #epoch = 0; #drag;
  #effect; #visibility; #onScreen=true; #lastTime;
  #canvasUsed=false;
  #rigReady=false; #following=false;
  #playing=false; #animationTime=0; #duration=0; #loop=true;
  #actorPlaying=false;
  #offhandHasMount=false;
  #scenePaused=false;
  #reported=false; #enabled=false; #attached=false; #read; #later; #hasPrimaryMount=false; #actionActive=false;
  #pending=new Map(); #seq=0; #busy=false; #dirty=false;
  #yaw=HOME.yaw; #pitch=HOME.pitch; #zoom=HOME.zoom;
  constructor() {
    super();
    const root = this.attachShadow({mode: "open"});
    root.innerHTML = `<style>
      :host{display:block;margin-block:.75rem;max-width:64rem;color:var(--ink,#25332f);font:inherit}:host([hidden]){display:none}
      figure{margin:0;background:var(--rail,#eef1ef);border:1px solid var(--line,#d7dfda);border-radius:.6rem;overflow:hidden}
      :host([framed]){margin:0;max-width:none}:host([framed]) figure{border:0;border-radius:0}
      canvas{display:block;width:100%;height:clamp(16rem,45vw,29rem);touch-action:none;cursor:grab;background:rgb(${STUDIO_RGB.join(" ")})}
      canvas:active{cursor:grabbing}canvas:focus-visible{outline:2px solid var(--accent,#17665d);outline-offset:-3px}
      figcaption{display:flex;flex-wrap:wrap;gap:.4rem 1rem;align-items:center;justify-content:space-between;padding:.45rem .8rem;border-top:1px solid var(--line,#d7dfda);font-size:.85rem;background:var(--paper,white)}
      nav{display:flex;align-items:center;gap:.25rem;margin-inline-start:auto}.playback{display:flex;align-items:center;gap:.25rem;flex-wrap:wrap}button{font:inherit;color:inherit;background:none;border:0;padding:.5rem;border-radius:.25rem;cursor:pointer}
      button:hover{background:var(--rail,#eef1ef)}output{min-width:3em;text-align:center;font-variant-numeric:tabular-nums}
      </style><figure><canvas tabindex="0" aria-label="装备模型，方向键旋转，加减键缩放，Home 复位"></canvas>
      <figcaption><div class="playback"><slot name="playback">拖动旋转 · 滚轮缩放</slot></div><nav aria-label="模型视角"><button data-step="-1" aria-label="缩小">−</button>
      <output>100%</output><button data-step="1" aria-label="放大">＋</button><button data-reset>复位</button></nav></figcaption></figure>`;
    const c = root.querySelector("canvas");
    this.#bindCanvas(c);
    root.querySelector("nav").onclick=e=>{
      if(e.target.hasAttribute("data-reset")) this.#reset();
      else if(e.target.dataset.step) this.#scale(1.12 ** Number(e.target.dataset.step));
    };
  }
  #bindCanvas(c){
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
  }
  #orbit(y,p){this.#yaw+=y;this.#pitch=Math.max(-1.35,Math.min(1.35,this.#pitch+p));this.frame();}
  #scale(f){this.#zoom=Math.min(1.8,Math.max(.65,this.#zoom*f));this.frame();}
  #reset(){this.#yaw=HOME.yaw;this.#pitch=HOME.pitch;this.#zoom=HOME.zoom;this.frame();}
  #resize(){
    const c=this.shadowRoot.querySelector("canvas"), box=c.getBoundingClientRect();
    const ratio=Math.min(devicePixelRatio||1,2.5,2560/Math.max(1,box.width,box.height));
    const w=Math.max(1,Math.round(box.width*ratio)),h=Math.max(1,Math.round(box.height*ratio));
    if(!this.#effect){
      if(c.width!==w)c.width=w;
      if(c.height!==h)c.height=h;
    }
    return [w,h];
  }
  async open(){
    this.dispose(); const epoch=this.#epoch;
    await init();
    if(epoch!==this.#epoch)throw Error("模型视图已关闭");
    this.#resize();
    this.#canvasUsed=true;
    const viewer=await EquipmentViewer.create(this.shadowRoot.querySelector("canvas"));
    if(epoch!==this.#epoch || !this.isConnected){viewer.free();throw Error("模型视图已关闭");}
    this.#viewer=viewer;
    this.#observe();
    this.#observeVisibility();
    return viewer;
  }
  /** After open(), load a weighted MDL with its matching SKLB and PAP record.
   * Set materials on the viewer as usual, then show() and playAnimation(). */
  setAnimatedModel(mdl,sklb,pap,record=0){
    if(!this.#viewer)throw Error("call open() first");
    this.#duration=this.#viewer.setAnimatedModel(mdl,sklb,pap,record);
    this.#animationTime=0;this.pauseAnimation();this.frame();return this.#duration;
  }
  seekAnimation(seconds){
    this.#viewer.seekAnimation(seconds);
    this.#animationTime=Math.max(0,Math.min(seconds,this.#duration));
    this.#wake();
  }
  playAnimation(loop=true){
    if(!this.#duration)return;
    if(this.#animationTime>=this.#duration)this.seekAnimation(0);
    this.#loop=loop;this.#playing=true;this.#wake();
  }
  pauseAnimation(){this.#playing=false;this.#lastTime=undefined;}
  #send(type, args={}, transfer=[]) {
    if(!this.#effect)return Promise.reject(Error("No weapon scene is open"));
    return new Promise((resolve,reject)=>{
      const id=++this.#seq;
      this.#pending.set(id,{resolve,reject});
      this.#effect.postMessage({type,id,...args},transfer);
    });
  }
  #progress(phase,done,total){
    this.dispatchEvent(new CustomEvent("loadprogress",{bubbles:true,detail:{phase,done,total}}));
  }
  async #entries(entries,epoch,phase,read=this.#read){
    if(entries.length>1024)throw Error("Too many resources in scene bundle");
    const it=entries.values(), result=[];
    let active=true,total=0;
    this.#progress(phase,0,entries.length);
    const run=async()=>{
      for(const [path,hash] of it){
        if(!active || epoch!==this.#epoch)throw Error("模型视图已关闭");
        const bytes=await read(path,hash);
        if(!(bytes instanceof Uint8Array)||bytes.byteLength>(64<<20)||(total+=bytes.byteLength)>(128<<20))
          throw Error(`Invalid or oversized resource bundle at ${path}`);
        result.push([path,bytes]);
        if(active && epoch===this.#epoch)this.#progress(phase,result.length,entries.length);
      }
    };
    try {await Promise.all(Array.from({length:4},run));} finally {active=false;}
    if(epoch!==this.#epoch)throw Error("模型视图已关闭");
    return result;
  }
  /** Owner and particles use the same scene, canvas, controls and camera. */
  async openWeapon(effect, read) {
    this.dispose(); const epoch=this.#epoch;
    this.#read=read;
    this.#hasPrimaryMount=effect.mounts?.length!==0;
    const owner=[];this.#later=[];
    for(const entry of Object.entries(effect.files)){
      (/(?:^|\/)vfx\/|\.(?:atex|avfx|sklb)$/i.test(entry[0])?this.#later:owner).push(entry);
    }
    const entries=await this.#entries(owner,epoch,"model-files");
    this.#progress("engine");
    this.#resize();
    const canvas=this.shadowRoot.querySelector("canvas").transferControlToOffscreen();
    this.#canvasUsed=true;
    const worker=this.#effect=new Worker(new URL("./effects/effect-worker.js",import.meta.url),{type:"module"});
    worker.onmessage=({data:{id,value,error,phase}})=>{
      if(phase){if(this.#pending.has(id))this.#progress(phase);return;}
      const call=this.#pending.get(id);this.#pending.delete(id);
      if(error)call?.reject(Error(error));else call?.resolve(value);
    };
    worker.onerror=e=>{
      const error=Error(e.message || "特效 Worker 加载失败");
      for(const call of this.#pending.values())call.reject(error);
      this.#pending.clear();
      this.dispatchEvent(new CustomEvent("rendererror",{detail:error}));
    };
    await this.#send("open",{canvas,entries,main:effect.main,yaw:this.#yaw,pitch:this.#pitch,
      distance:orbitDistance(this.#zoom,canvas.width,canvas.height)},[canvas]);
    this.#observe();
    this.#observeVisibility();
    this.show();
  }
  #observeVisibility(){
    this.#visibility=new IntersectionObserver(([e])=>{this.#onScreen=e.isIntersecting;this.#wake();});
    this.#visibility.observe(this);
    document.addEventListener("visibilitychange",this.#wake);
  }
  async setEffectEnabled(enabled){
    if(enabled && this.#hasPrimaryMount && !this.#attached){
      const entries=await this.#entries(this.#later,this.#epoch,"effect-files");
      this.#progress("unpack-effect");
      await this.#send("attach",{entries});
      this.#attached=true;
      this.#rigReady=false;
    }
    await this.#send("residents",{enabled,replay:enabled});
    this.#enabled=enabled;this.#wake();
  }
  async openEffect(effect,read){await this.openWeapon(effect,read);await this.setEffectEnabled(true);}
  /** Supply Apricot scene parameters and the tone texture's red sample at its center. */
  async setParticleTone(tX, tW, centerRed) {
    await this.#send("particle-tone", {tX, tW, centerRed});
    this.frame();
  }
  /** Install an independent export (including --owner) on the actor's n_buki_l. */
  async setOffhandBundle(manifest, read=this.#read, {grip=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}={}) {
    if(!this.#effect) throw Error("No weapon scene is open");
    const {main, owner, mounts, files}=manifest;
    const paths=Object.entries(files??{});
    if(typeof main!=="string" || !/^\d+$/.test(main) || BigInt(main)>0xffffffffffffffffn ||
       typeof owner!=="string" || !Object.hasOwn(files??{},owner) || !Array.isArray(mounts) ||
       mounts.length>1 || mounts.some(m=>!Object.hasOwn(files,m)) || paths.length>1024 ||
       paths.some(([,h])=>typeof h!=="string" || !/^[0-9a-f]{64}$/.test(h)) ||
       grip?.length!==16 || !Array.from(grip).every(Number.isFinite))
      throw Error("Invalid offhand bundle or grip");
    const entries=await this.#entries(paths,this.#epoch,"offhand-files",read);
    await this.#send("set-offhand",{main,owner,mounts,entries,grip});
    this.#offhandHasMount=mounts.length>0;
    if(mounts.length && !this.#enabled){await this.#send("residents",{enabled:true,replay:false});this.#enabled=true;}
    this.frame();
  }
  async clearOffhand(){ await this.#send("clear-offhand");this.#offhandHasMount=false;this.frame(); }
  /** Keep the prepared owner rig for actor grip and actions without drawing the weapon. */
  async setOwnerVisible(visible){ await this.#send("owner-visible",{visible:!!visible}); this.frame(); }
  /** Set the orbit zoom shown by the tool bar; range matches the view controls. */
  setZoom(zoom){ this.#zoom=Math.min(1.8,Math.max(.65,Number(zoom)||1)); this.frame(); }
  async setOffhandVisible(visible){ await this.#send("offhand-visible",{visible:!!visible}); this.frame(); }
  triggerOffhand(scheduler, trigger){ return this.#send("offhand-trigger",{scheduler,trigger}); }
  offhandDiagnostics(){ return this.#send("offhand-diagnostics"); }
  /** Play a --pack manifest; read(logicalPath,digest) resolves its CAS bytes.
   * motion=false plays only C012; otherwise C010 drives the selected skeleton. */
  async playActionBundle(bundle,preview,read=this.#read,motion={}){
    if(!this.#effect)throw Error("No weapon scene is open");
    const animate=motion!==false&&bundle.clips.length>0;
    const paths=new Set(Object.values(bundle.paps));
    if(animate&&(!bundle.sklb||bundle.unresolvedClips.length))
      throw Error("Missing actor skeleton or named PAP: "+bundle.unresolvedClips.join(", "));
    const isMotion=p=>p===bundle.sklb||paths.has(p);
    const entries=await this.#entries(Object.entries(bundle.files).filter(([p])=>animate||!isMotion(p)),
      this.#epoch,"action-files",read);
    const files=new Map(entries);
    const options=animate?{bone:"n_buki_r",grip:[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],...motion,
      sklb:files.get(bundle.sklb),paps:Object.fromEntries(Object.entries(bundle.paps).map(([n,p])=>[n,files.get(p)]))}:undefined;
    return this.playAction({effects:bundle.effects,clips:animate?bundle.clips:[]},preview,
      entries.filter(([p])=>!isMotion(p)),options);
  }
  /** Start an imported C012 preview on an open owner; entries are [logicalPath, Uint8Array]. */
  async playAction(events, preview, entries, motion){
    if(motion&&!this.#rigReady)await this.weaponSkeleton();
    const warnings=await this.#send("play-action",{events,preview,entries,motion});
    this.#actionActive=true;this.#reported=false;this.#wake();
    return warnings;
  }
  async stopAction(){
    if(!this.#actionActive)return;
    await this.#send("stop-action");
    this.#actionActive=false;this.#wake();
  }
  async setActionElements(selector,elements){
    await this.#send("action-elements",{selector,elements});this.frame();
  }
  async setDirectionalGoal(frame){
    await this.#send("directional-goal",{frame});this.frame();
  }
  triggerAction(occurrence,scheduler,trigger){
    return this.#send("action-trigger",{occurrence,scheduler,trigger});
  }
  async setActionController(input){
    await this.#send("action-controller",{input});this.frame();
  }
  actionControllerStatus(occurrence){return this.#send("controller-status",{occurrence});}
  async endActionCommands(){
    await this.#send("end-action-commands");this.#wake();
  }
  fadeAction(occurrence,scheduler,frames,mode,keep=false){
    return this.#send("action-fade",{occurrence,scheduler,frames,mode,keep});
  }
  triggerResident(owner,scheduler,trigger){
    return this.#send("resident-trigger",{owner,scheduler,trigger});
  }
  /** Replace a C012 second endpoint in world space. Live Documents use the
   * latest object and ElementIds on their next advancing simulation tick. */
  async setActionTarget(binding){
    await this.#send("action-target",{binding});this.frame();
  }
  async replayEffect(seed){
    if(seed!==undefined && (seed.length!==4 || !Array.from(seed).every(x=>Number.isInteger(x)&&x>=0&&x<=0xffffffff)))
      throw new RangeError("expected four unsigned 32-bit RNG words");
    if(this.#enabled||this.#actionActive||seed!==undefined){await this.#send("reset",{seed});this.#wake();}
  }
  randomState(){return this.#send("random-state");}
  async setActorPlacement(position,rotation=[0,0,0,1]){
    await this.#send("actor-placement",{position,rotation});this.frame();
  }
  /** Freeze all scene clocks; camera interaction still requests a zero-delta redraw. */
  pauseScene(){this.#scenePaused=true;this.#lastTime=undefined;}
  resumeScene(){this.#scenePaused=false;this.#wake();}
  /** Advance a paused weapon/actor/action scene by exact 60 Hz ticks. */
  async stepScene(ticks=1){
    if(!this.#loaded||!this.#effect)throw Error("No weapon scene is open");
    if(!Number.isInteger(ticks)||ticks<1||ticks>6)throw Error("stepScene expects 1 to 6 ticks");
    this.pauseScene();
    const [w,h]=this.#resize();
    const result=await this.#send("frame",{w,h,yaw:this.#yaw,pitch:this.#pitch,
      distance:orbitDistance(this.#zoom,w,h),dt:ticks/60,enabled:this.#enabled||this.#actionActive,report:true});
    const warnings=Array.isArray(result)?result:result?.warnings;
    if(warnings?.length)this.dispatchEvent(new CustomEvent("renderwarning",{detail:warnings}));
    return result;
  }
  /** Load an export_actor manifest; read(logicalPath,sha256) returns verified bytes. */
  async setActorBundle(manifest,read=this.#read){
    if(!this.#effect)throw Error("No weapon scene is open");
    const {files,customize,equipment,sklb,pap,record}=manifest;
    const paths=Object.entries(files??{});
    if(typeof sklb!=="string"||!sklb.endsWith(".sklb")||typeof pap!=="string"||!pap.endsWith(".pap")||
       !Object.hasOwn(files??{},sklb)||!Object.hasOwn(files??{},pap)||
       !Number.isInteger(record)||record<0||record>65535||
       !Array.isArray(customize)||customize.length!==26||
       !customize.every(x=>Number.isInteger(x)&&x>=0&&x<=255)||
       !Array.isArray(equipment)||equipment.length>16||
       paths.length>1024||paths.some(([,sha])=>typeof sha!=="string"||!/^[0-9a-f]{64}$/.test(sha)))
      throw Error("Invalid actor manifest");
    const entries=await this.#entries(paths,this.#epoch,"actor-files",read);
    const byPath=new Map(entries);
    await this.setActor(entries.filter(([p])=>p!==pap),Uint8Array.from(customize),
      byPath.get(sklb),byPath.get(pap),{equipment,record});
  }

  /** Draw a dressed actor with the current weapon. entries contain logical
   * body, equipment, SKLB, material and texture files; modelMain/Sub are strings. */
  async setActor(entries,customize,sklb,pap,{equipment=[],record=0,bone="n_buki_r",loop=true,overlay,
    grip=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}={}){
    if(!this.#rigReady)await this.weaponSkeleton();
    await this.#send("setActor",{entries,customize,sklb,pap,equipment,record,bone,loop,overlay,grip});
    this.#actorPlaying=true;this.#following=false;this.#wake();
  }
  async clearActor(){
    if(!this.#actorPlaying)return;
    await this.#send("clearActor");this.#actorPlaying=this.#offhandHasMount=false;this.#wake();
  }
  actorPartialRigs(){return this.#send("actorPartialRigs");}
  actorPartialSkeleton(path){return this.#send("actorPartialSkeleton",{path});}
  async setActorPartialPose(path,locals){await this.#send("setActorPartialPose",{path,locals});this.frame();}
  async setActorPartialClip(path,pap,name,looping=true){
    const duration=await this.#send("setActorPartialClip",{path,pap,name,looping});this.#wake();return duration;
  }
  async clearActorPartial(path){await this.#send("clearActorPartial",{path});this.frame();}
  /** Owner bone names and packed parent-local rest transforms (T3, quaternion XYZW, S3). */
  async weaponSkeleton(){
    if(!this.#effect)throw Error("No weapon scene is open");
    const entries=!this.#attached&&!this.#rigReady
      ? await this.#entries(this.#later.filter(([p])=>p.endsWith(".sklb")),this.#epoch,"skeleton-files")
      : undefined;
    const skeleton=await this.#send("weaponSkeleton",{entries});
    this.#rigReady=skeleton.names.length>0;
    return skeleton;
  }
  async setWeaponPose(trs){
    if(!this.#rigReady)await this.weaponSkeleton();
    await this.#send("weaponPose",{trs});this.#following=false;this.frame();
  }
  /** Sample a character hand on every VFX tick, including the birth frame.
   * grip is a column-major weapon-to-hand matrix supplied by the host. */
  async followWeaponHand(sklb,pap,{record=0,bone="n_buki_r",loop=true,overlay,weaponClip,
    grip=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1]}={}){
    if(!this.#rigReady)await this.weaponSkeleton();
    await this.#send("followHand",{sklb,pap,record,bone,loop,grip,overlay,weaponClip});this.#following=true;this.#wake();
  }
  #wake=()=>{this.#lastTime=undefined;this.frame();};
  #observe(){
    this.#observer=new ResizeObserver(()=>this.frame());
    this.#observer.observe(this.shadowRoot.querySelector("canvas"));
    window.addEventListener("resize",this.frame);
  }
  frame=()=>{
    if(this.#busy){this.#dirty=true;return;}
    if((!this.#viewer && !this.#effect) || this.#raf) return;
    this.#raf=requestAnimationFrame(async now=>{
      const epoch=this.#epoch;
      this.#raf=0;const [w,h]=this.#resize();
      this.shadowRoot.querySelector("output").textContent=`${Math.round(this.#zoom*100)}%`;
      if(this.#loaded) {
        try {
          if(this.#effect){
            if(this.hidden || document.hidden || !this.#onScreen){this.#lastTime=undefined;return;}
            this.#busy=true;this.#dirty=false;
            const result=await this.#send("frame",{w,h,yaw:this.#yaw,pitch:this.#pitch,
              distance:orbitDistance(this.#zoom,w,h),dt:this.#scenePaused||this.#lastTime===undefined?0:(now-this.#lastTime)/1000,
              enabled:this.#enabled||this.#actionActive,report:(this.#enabled||this.#actionActive)&&!this.#reported});
            if(epoch!==this.#epoch)return;
            this.#busy=false;
            const warnings=Array.isArray(result)?result:result?.warnings;
            if(warnings){
              this.#reported=true;
              if(warnings.length)this.dispatchEvent(new CustomEvent("renderwarning",{detail:warnings}));
            }
            this.#lastTime=this.#scenePaused?undefined:now;
            if(this.#dirty||(!this.#scenePaused&&(this.#actorPlaying||this.#following
              ||(this.#enabled&&(this.#attached||this.#offhandHasMount))||(this.#actionActive&&!result?.finished))))this.frame();
          }else{
            if(this.#playing){
              if(this.hidden || document.hidden || !this.#onScreen){this.#lastTime=undefined;return;}
              const next=this.#animationTime+(this.#lastTime===undefined?0:(now-this.#lastTime)/1000);
              this.#animationTime=this.#loop?next%this.#duration:Math.min(next,this.#duration);
              this.#viewer.seekAnimation(this.#animationTime);
              this.#lastTime=now;
              if(!this.#loop && next>=this.#duration)this.pauseAnimation();
            }
            this.#viewer.setBackground(...STUDIO_RGB);
            this.#viewer.drawZoom(this.#yaw,this.#pitch,this.#zoom);
            if(this.#playing)this.frame();
          }
        }
        catch(error){if(epoch===this.#epoch){this.#busy=false;this.dispatchEvent(new CustomEvent("rendererror",{detail:error}));}}
      }
    });
  };
  #loaded=false;
  show(){this.#loaded=true;this.frame();}
  dispose(){
    this.#scenePaused=this.#actionActive=this.#hasPrimaryMount=false;
    this.#epoch++;this.#loaded=false;
    cancelAnimationFrame(this.#raf);this.#raf=0;
    window.removeEventListener("resize",this.frame);this.#observer?.disconnect();
    this.#viewer?.free();this.#viewer=undefined;
    this.pauseAnimation();this.#duration=this.#animationTime=0;
    this.#actorPlaying=this.#offhandHasMount=false;
    this.#visibility?.disconnect();document.removeEventListener("visibilitychange",this.#wake);
    this.#effect?.terminate();this.#effect=undefined;
    if(this.#canvasUsed){
      // A GPU canvas cannot switch context type or transfer ownership twice.
      const previous=this.shadowRoot.querySelector("canvas"), next=previous.cloneNode(false);
      previous.replaceWith(next);this.#bindCanvas(next);this.#canvasUsed=false;this.#drag=null;
    }
    for(const call of this.#pending.values())call.reject(Error("模型视图已关闭"));
    this.#pending.clear();this.#read=this.#later=undefined;
    this.#busy=this.#dirty=this.#enabled=this.#attached=this.#reported=false;this.#lastTime=undefined;this.#onScreen=true;
    this.#rigReady=this.#following=false;
  }
  disconnectedCallback(){this.dispose();}
}
if (!customElements.get("icexiv-equipment-canvas")) customElements.define("icexiv-equipment-canvas",EquipmentCanvas);
