import init, {DressedActor, EffectFiles, WeaponEffectViewer, c010_windows} from "./icexiv_effects.js";

let canvas, viewer, ownerFiles, clip;
let actorClip;
let actionWarningCount;
let sequence;
function clearSequence() {
  if (!sequence) return;
  viewer.clear_action_hand();
  sequence.free();
  sequence = undefined;
}
async function prepareSequence(clips, previewJson, options) {
  if (!Array.isArray(clips)) throw Error("Expected a C010 clip list");
  if (!clips.length) return {warnings:[]};
  const [windows, warnings] = JSON.parse(c010_windows(JSON.stringify(clips), previewJson));
  if (!windows.length) return {warnings};
  if (!options) throw Error("Active C010 requires a motion resource mapping");
  const {sklb, paps, bone, gapHand, grip} = options;
  const actor = options.target==="actor";
  if(options.target!==undefined && !actor && options.target!=="hand")throw Error("Unknown C010 pose target");
  const names=actor?viewer.actor_bone_names():undefined;
  const restLocals=actor?viewer.actor_rest_locals():undefined;
  const gapLocals=actor&&(options.gap==="rest"||options.gapLocals===undefined)?restLocals:options.gapLocals;
  const matrix = (m) => m?.length===16 && Array.from(m).every(Number.isFinite);
  if (actor && (!names.length || restLocals?.length!==names.length*10 || gapLocals?.length!==names.length*10)) throw Error("Actor C010 requires an attached actor and gapLocals or gap:'rest'");
  if (!actor && (!bone || (gapHand!==undefined&&!matrix(gapHand)) || !matrix(grip))) throw Error("Invalid C010 bone, gapHand or grip");
  const module = await import("../icexiv_renderer.js");
  await module.default();
  const decoded = new Map();
  const free = () => {for (const c of decoded.values()) c.free(); decoded.clear();};
  let bytes=0;
  try {
    const load=(name,key)=>{
      if (decoded.has(key))return;
      if (!paps || !Object.hasOwn(paps,name)) throw Error(`Missing C010 PAP: ${name}`);
      bytes+=paps[name].byteLength;
      if (decoded.size>=64 || bytes>128*1024*1024) throw Error("C010 clip resource budget exceeded");
      const c = module.SkeletonClip.loadNamed(sklb, paps[name], name);
      decoded.set(key,c);
      if(actor)c.retargetedLocalPose(names,restLocals,0);else c.boneWorld(bone,0);
    };
    for (const [i,w] of windows.entries()) {
      load(w.name,w.name);
      // A self-transition has two distinct local clocks; sampling a single
      // WASM object for both sides would overwrite the outgoing pose.
      if(i && windows[i-1].end>w.start && windows[i-1].name===w.name)load(w.name,w.occurrence);
    }
    const gap = actor ? Float32Array.from(gapLocals) : gapHand===undefined
      ? decoded.values().next().value.restBoneWorld(bone) : Float32Array.from(gapHand);
    const indexAt=t=>windows.findIndex(w => w.start<=t && t<w.end);
    const windowAt=t=>windows[indexAt(t)];
    const sample = t => {
      const i=indexAt(t),w=windows[i],next=windows[i+1];
      if (!w) return gap;
      const c = decoded.get(w.name);
      const age=Math.min(t-w.start,c.durationSeconds());
      c.sample(age);
      if(next && t>=next.start){
        const incoming=decoded.get(w.name===next.name?next.occurrence:next.name);
        incoming.sample(Math.min(t-next.start,incoming.durationSeconds()));
        c.blendCurrent(incoming,(t-next.start)/(w.end-next.start));
      }
      return actor?c.currentRetargetedLocalPose(names,restLocals):c.currentBoneWorld(bone);
    };
    return {warnings, free, sample, grip, actor, windowAt, end:windows.at(-1).end};
  } catch(e) {free();throw e;}
}
function bundle(entries) {
  const files = new EffectFiles();
  try {
    for (const [path, bytes] of entries) files.add(path, bytes);
    return files;
  } catch (error) { files.free(); throw error; }
}
let queue=Promise.resolve();
self.onmessage = ({data:m}) => {queue=queue.then(()=>dispatch(m));};
async function dispatch(m) {
  try {
    let value;
    switch (m.type) {
      case "open":
        await init();
        canvas = m.canvas;
        ownerFiles = m.entries;
        self.postMessage({id:m.id,phase:"unpack-model"});
        viewer = await WeaponEffectViewer.create(canvas, BigInt(m.main), bundle(ownerFiles));
        self.postMessage({id:m.id,phase:"first-frame"});
        viewer.frame(m.yaw,m.pitch,m.distance,0,false);
        await viewer.submitted();
        break;
      case "openScene":
        await init();
        canvas = m.canvas;
        ownerFiles = null;
        viewer = await WeaponEffectViewer.create_scene(canvas);
        break;
      case "setPan":
        viewer.set_pan(m.x, m.y);
        break;
      case "addProp":
        viewer.add_prop(m.propId, BigInt(m.main), m.bone, m.grip, bundle(m.entries));
        break;
      case "setPropVisible":
        viewer.set_prop_visible(m.propId, !!m.visible);
        break;
      case "clearProps":
        viewer.clear_props();
        break;
      case "setActionOccurrenceEnabled":
        viewer.set_action_occurrence_enabled(m.occurrence, !!m.enabled);
        break;
      case "seekAction":
        viewer.seek_action(m.seconds);
        break;
      case "attach":
        viewer.attach(bundle([...ownerFiles, ...m.entries]));
        ownerFiles = null;
        break;
      case "residents":
        viewer.show_residents(m.enabled,m.replay);break;
      case "reset":
        if(m.seed === undefined) viewer.reset();
        else viewer.replay_with_random_seed(m.seed);
        if (actionWarningCount !== undefined) actionWarningCount = 0;
        break;
      case "random-state": value=Array.from(viewer.random_state());break;
      case "actor-placement": viewer.set_actor_placement(m.position,m.rotation);break;
      case "action-trigger": value=viewer.trigger_action(m.occurrence,m.scheduler,m.trigger);break;
      case "action-controller": viewer.set_controller_input(JSON.stringify(m.input));break;
      case "controller-status": value=JSON.parse(viewer.controller_status(m.occurrence));break;
      case "end-action-commands": clearSequence();viewer.end_action_commands();break;
      case "action-fade": value=viewer.fade_action(m.occurrence,m.scheduler,m.frames,m.mode,m.keep);break;
      case "resident-trigger": value=viewer.trigger_resident(m.owner,m.scheduler,m.trigger);break;
      case "particle-tone":
        viewer.set_particle_tone(m.tX, m.tW, m.centerRed);
        break;
      case "set-offhand":
        viewer.set_offhand(BigInt(m.main),m.owner,JSON.stringify(m.mounts),bundle(m.entries),m.grip);
        break;
      case "clear-offhand":
        viewer.clear_offhand();
        break;
      case "owner-visible":
        viewer.set_owner_visible(m.visible);break;
      case "offhand-visible":
        viewer.set_offhand_visible(m.visible);
        break;
      case "offhand-trigger":
        value=viewer.trigger_offhand(m.scheduler,m.trigger);
        break;
      case "offhand-diagnostics":
        value=viewer.offhand_diagnostics();
        break;
      case "play-action": {
        const manifest = Array.isArray(m.events)?{effects:m.events,clips:[]}:m.events;
        // Both admissions use the same host variable/override snapshot.
        const previewJson = JSON.stringify(m.preview);
        const next = await prepareSequence(manifest.clips,previewJson,m.motion);
        try {
          viewer.play_action(JSON.stringify(manifest.effects), previewJson, bundle(m.entries));
          clearSequence();
          if(next.sample) {
            const selector=JSON.stringify(m.motion.weaponBinding??null);
            try {if(next.actor)viewer.follow_action_actor(next.sample,selector);
              else viewer.follow_action_hand(next.sample,next.grip,selector);}
            catch(e) {viewer.stop_action();actionWarningCount=undefined;throw e;}
            sequence=next;
          }
          value = viewer.diagnostics();
          actionWarningCount = viewer.action_warning_count();
          value.push(...next.warnings);
        } catch(e) {if(sequence!==next)next.free?.();throw e;}
        break;
      }
      case "stop-action": clearSequence();viewer.stop_action(); actionWarningCount = undefined; break;
      case "action-elements": viewer.set_action_elements(JSON.stringify(m.selector),JSON.stringify(m.elements));break;
      case "directional-goal": viewer.set_directional_goal(JSON.stringify(m.frame));break;
      case "action-target": viewer.set_action_target(JSON.stringify(m.binding));break;
      case "weaponSkeleton":
        if(m.entries)viewer.prepare_skeleton(bundle(m.entries));
        value = { names: viewer.bone_names(), rest: viewer.rest_local_pose() };
        break;
      case "setActor": {
        clearSequence();
        const motion=await import("../icexiv_renderer.js");
        await motion.default();
        const c=m.overlay ? motion.SkeletonClip.loadLayered(m.sklb,m.pap,m.record,
          m.overlay.pap,m.overlay.record??0,m.overlay.weight??1) : motion.SkeletonClip.load(m.sklb,m.pap,m.record);
        let actor;
        try {
          actor=DressedActor.load(bundle(m.entries),m.customize,JSON.stringify(m.equipment));
          const names=actor.bone_names(), restLocals=actor.rest_locals(), duration=c.durationSeconds();
          const sample=t=>c.retargetedLocalPose(names,restLocals,m.loop&&duration>0?t%duration:Math.min(t,duration));
          const owned=actor;
          actor=undefined;
          viewer.set_actor(owned,m.bone,m.grip,sample);
        } catch(e) {c.free();throw e;}
        finally {actor?.free();}
        actorClip?.free();actorClip=c;clip?.free();clip=undefined;
        break;
      }
      case "clearActor":
        clearSequence();viewer.clear_actor();actorClip?.free();actorClip=undefined;break;
      case "actorPartialRigs": value=viewer.actor_partial_rigs();break;
      case "actorPartialSkeleton": value={names:viewer.actor_partial_names(m.path),rest:viewer.actor_partial_rest(m.path)};break;
      case "setActorPartialPose": viewer.set_actor_partial_pose(m.path,m.locals);break;
      case "setActorPartialClip": value=viewer.set_actor_partial_clip(m.path,m.pap,m.name,m.looping);break;
      case "clearActorPartial": viewer.clear_actor_partial(m.path);break;
      case "weaponPose":
        viewer.set_local_pose(m.trs);sequence?.free();sequence=undefined;clip?.free();clip=undefined;break;
      case "followHand": {
        const motion = await import("../icexiv_renderer.js");
        await motion.default();
        const next = m.overlay ? motion.SkeletonClip.loadLayered(m.sklb,m.pap,m.record,
          m.overlay.pap,m.overlay.record??0,m.overlay.weight??1) : motion.SkeletonClip.load(m.sklb, m.pap, m.record);
        let weapon;
        const free=()=>{weapon?.free();next.free();};
        try {
          const duration = next.durationSeconds();
          const time=(t,d)=>m.loop&&d>0 ? t%d : Math.min(t,d);
          if(m.weaponClip)weapon=motion.SkeletonClip.load(m.weaponClip.sklb,m.weaponClip.pap,m.weaponClip.record??0);
          const names=viewer.bone_names(), weaponDuration=weapon?.durationSeconds();
          viewer.follow_hand(t => {
            const hand=next.boneWorld(m.bone,time(t,duration));
            if(!weapon)return hand;
            const locals=weapon.localPose(names,time(t,weaponDuration));
            const packet=new Float32Array(16+locals.length);
            packet.set(hand);packet.set(locals,16);return packet;
          }, m.grip);
        } catch(e) {free();throw e;}
        sequence?.free();sequence=undefined;clip?.free();clip={free};break;
      }
      case "frame":
        if (canvas.width !== m.w) canvas.width = m.w;
        if (canvas.height !== m.h) canvas.height = m.h;
        viewer.frame(m.yaw, m.pitch, m.distance, m.dt, m.enabled);
        if (actionWarningCount !== undefined) {
          const count = viewer.action_warning_count();
          const age=viewer.preview_seconds();
          value = {age, finished: viewer.action_finished() && (!sequence || age>=sequence.end), warnings: count > actionWarningCount
            ? viewer.action_warnings_since(actionWarningCount) : undefined};
          if(m.report && sequence)value.clip=sequence.windowAt(age)??null;
          actionWarningCount = count;
        } else if (m.report) value = viewer.diagnostics();
        await viewer.submitted();
        break;
    }
    self.postMessage({id:m.id, value});
  } catch (error) { self.postMessage({id:m.id, error:`${m.type}: ${error.message ?? error}`}); }
}
