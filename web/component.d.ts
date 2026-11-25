import type { EquipmentViewer } from "./icexiv_renderer.js";

export type ResourceReader = (path: string, sha256: string) => Uint8Array | Promise<Uint8Array>;
export type Entries = [string, Uint8Array][];
export type Files = Record<string, string>;
/** Column-major 4×4. */
export type Matrix = Float32Array | number[];
export type XYZ = [number, number, number];
export type Basis = [XYZ, XYZ, XYZ];
export interface WeaponManifest { main: string; owner: string; mounts: string[]; files: Files; diagnostics?: string[] }
export interface EquipmentPiece { itemId?: number; modelMain: string; modelSub?: string; equipSlotCategory: number; stainIds?: [number, number] }
export interface ActorManifest { customize: number[]; equipment: EquipmentPiece[]; sklb: string; pap: string; record: number; files: Files }
export interface Target { origin: number; kind: number; id: number }
export interface ElementFrame { id: number; position: XYZ; basis: Basis }
export interface Binding { target: Target; position: XYZ; quaternion?: [number, number, number, number]; scale?: XYZ; elements?: ElementFrame[] }
export type DirectionalGoal = { occurrence: number; binder: number; ordinal: number } &
  ({ basis: Basis; position: XYZ; missing?: never } | { missing: true; basis?: never; position?: never });
export interface ActionPreview {
  controllers?: ControllerInput[];
  units_per_second: number;
  bindings: Binding[];
  enabled_tracks?: [string, number][];
  /** Resolved integer TMTR variable words; nonempty enabled_tracks overrides this evaluator. */
  condition_inputs?: {variables?: Record<string, number>; actors?: Record<string, Record<string, Record<string, number>>>};
  crossfade_clips?: boolean;
  command_frames?: {occurrence: number; position: XYZ; basis: Basis; scale: XYZ}[];
  directional_goals?: DirectionalGoal[];
}
export interface ControllerInput {
  occurrence: number; allocation_available?: boolean; eligible?: boolean | null;
  context_code?: number; state_32?: boolean; manager_ready?: boolean;
}
export interface ControllerStatus {
  phase: "waiting"|"allocation_failed"|"ineligible"|"state_blocked"|"attached"|"detached"|"released"|"skipped";
  external: boolean; trigger_requests: number; trigger_attached: boolean; state_32_stop: boolean;
}
export interface Link {
  tmb: string; actor: number; track: number; command: number;
  actor_start: number; track_start: number; start: number; duration: number; ability_delay: number;
  conditions: [number,number,number][] | null;
}
export interface EffectCommand {
  chain: Link[]; avfx: string; targets: [Target, Target]; scale: XYZ; position: XYZ;
  rotation_radians: XYZ; color: [number, number, number, number]; visibility: number;
}
export interface ActionCommands {effects: EffectCommand[]; clips: {name: string; chain: Link[]}[]}
export interface ActionBundle extends ActionCommands {
  action: number; timeline: number; entry: string; files: Files; sklb: string | null;
  paps: Record<string,string>; unresolvedClips: string[];
}
export interface MotionOptions {
  target?: "actor" | "hand"; bone?: string; grip?: Matrix; weaponBinding?: Target;
  gapHand?: Matrix; gapLocals?: Float32Array | number[]; gap?: "rest";
}
export interface ClipOptions {
  record?: number; bone?: string; loop?: boolean; grip?: Matrix;
  overlay?: {pap: Uint8Array; record?: number; weight?: number};
}
export class EquipmentCanvas extends HTMLElement {
  open(): Promise<EquipmentViewer>;
  setAnimatedModel(mdl: Uint8Array, sklb: Uint8Array, pap: Uint8Array, record?: number): number;
  seekAnimation(seconds: number): void;
  playAnimation(loop?: boolean): void;
  pauseAnimation(): void;
  openWeapon(manifest: WeaponManifest, read: ResourceReader): Promise<void>;
  openEffect(manifest: WeaponManifest, read: ResourceReader): Promise<void>;
  setEffectEnabled(enabled: boolean): Promise<void>;
  setParticleTone(tX: number, tW: number, centerRed: number): Promise<void>;
  setActorBundle(manifest: ActorManifest, read?: ResourceReader): Promise<void>;
  setActor(entries: Entries, customize: Uint8Array, sklb: Uint8Array, pap: Uint8Array,
    options?: ClipOptions & {equipment?: EquipmentPiece[]}): Promise<void>;
  clearActor(): Promise<void>;
  actorPartialRigs(): Promise<string[]>;
  actorPartialSkeleton(path: string): Promise<{names: string[]; rest: Float32Array}>;
  setActorPartialPose(path: string, locals: Float32Array | number[]): Promise<void>;
  setActorPartialClip(path: string, pap: Uint8Array, name: string, looping?: boolean): Promise<number>;
  clearActorPartial(path: string): Promise<void>;
  setOffhandBundle(manifest: WeaponManifest, read?: ResourceReader, options?: {grip?: Matrix}): Promise<void>;
  clearOffhand(): Promise<void>;
  setOwnerVisible(visible: boolean): Promise<void>;
  setZoom(zoom: number): void;
  setOffhandVisible(visible: boolean): Promise<void>;
  offhandDiagnostics(): Promise<string[]>;
  followWeaponHand(sklb: Uint8Array, pap: Uint8Array, options?: ClipOptions & {
    weaponClip?: {sklb: Uint8Array; pap: Uint8Array; record?: number}
  }): Promise<void>;
  weaponSkeleton(): Promise<{names: string[]; rest: Float32Array}>;
  setWeaponPose(trs: Float32Array | number[]): Promise<void>;
  playActionBundle(bundle: ActionBundle, preview: ActionPreview, read?: ResourceReader,
    motion?: MotionOptions | false): Promise<string[]>;
  playAction(commands: ActionCommands | EffectCommand[], preview: ActionPreview, entries: Entries,
    motion?: MotionOptions & {sklb: Uint8Array; paps: Record<string,Uint8Array>}): Promise<string[]>;
  stopAction(): Promise<void>;
  endActionCommands(): Promise<void>;
  setActionController(input: ControllerInput): Promise<void>;
  actionControllerStatus(occurrence: number): Promise<ControllerStatus | null>;
  setActionElements(selector: Target, elements: ElementFrame[]): Promise<void>;
  setActionTarget(binding: Binding): Promise<void>;
  setDirectionalGoal(frame: DirectionalGoal): Promise<void>;
  triggerAction(occurrence: number, scheduler: number, trigger: number): Promise<boolean>;
  fadeAction(occurrence: number, scheduler: number, frames: number, mode: 0|1|2|3, keep?: boolean): Promise<boolean>;
  triggerResident(owner: string, scheduler: number, trigger: number): Promise<boolean>;
  triggerOffhand(scheduler: number, trigger: number): Promise<boolean>;
  replayEffect(seed?: [number, number, number, number] | Uint32Array): Promise<void>;
  randomState(): Promise<[number, number, number, number]>;
  setActorPlacement(position: XYZ, rotation?: [number, number, number, number]): Promise<void>;
  pauseScene(): void;
  resumeScene(): void;
  stepScene(ticks?: number): Promise<undefined | string[] | {
    finished: boolean; warnings?: string[]; clip?: {start:number;end:number;name:string;occurrence:number}|null
  }>;
  frame(): void;
  show(): void;
  dispose(): void;
}
declare global { interface HTMLElementTagNameMap { "icexiv-equipment-canvas": EquipmentCanvas } }
