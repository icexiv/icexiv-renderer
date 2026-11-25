# 骨骼动作与动态挂点

装备蒙皮和武器特效使用两种不同的矩阵。蒙皮使用「当前骨世界矩阵 × 绑定姿态世界矩阵的逆」；挂点使用「当前骨世界矩阵 × MDL ElementId 局部变换」。把蒙皮矩阵当成挂点矩阵会丢失绑定姿态下的骨位置。

## 播放装备动作

```js
const viewer = await element.open();
const seconds = element.setAnimatedModel(mdl, sklb, pap, record);
// 按普通装备的方式在 viewer 上设置材质槽。
element.show();
element.playAnimation(true);
element.pauseAnimation();
element.seekAnimation(seconds / 2);
```

`animationRecords(pap)` 给出 PAP 文件中的记录名，数组下标就是 `record`。播放器通过该记录的 Havok binding 将轨道映射到所选 SKLB；MDL 顶点的混合索引还要经过各自网格的骨表。挂点和骨姿态都保留游戏资产单位，轨道相机的自动居中不写回骨架。可在 `examples/viewer.html` 同时选择 MDL、SKLB、PAP 查看和切换动作。

没有画布时可直接取骨帧。时间单位为秒，矩阵为列主序，`boneWorld` 的时间必须落在 clip 时长内。

```js
import init, {SkeletonClip} from "./pkg/icexiv_renderer.js";
await init();
const clip = SkeletonClip.load(sklb, pap, record);
const hand = clip.boneWorld("n_buki_r", clip.durationSeconds() / 2);
clip.free();
```

`loadLayered(sklb, basePap, baseRecord, overlayPap, overlayRecord, weight)` 接受普通基础层和一条加法层，权重范围 `[0,1]`。两条轨道使用同一采样时钟。使用匹配的骨架和显式轨道映射；这里没有隐式的跨种族动作重定向。

同场着装角色的基础骨架与面部、头发的局部骨架分别求值。`await element.actorPartialRigs()` 枚举实际使用的 SKLB 路径；以完整路径调用 `setActorPartialClip(path, pap, name)`、`setActorPartialPose(path, trs)` 或 `clearActorPartial(path)`。例如 c0101 的 `cfxb_blink1` 可以叠在身体动作上。同名关节不会跨局部骨架串用，清除面部姿态不会重播身体或武器。这里驱动的是骨架，不模拟风或头发物理。

跨动作过渡可分别 `a.sample(tA)`、`b.sample(tB)`，再 `a.blendCurrent(b, weight, subtree)`，用 `a.currentLocalPose(names)` 或 `a.currentBoneWorld(name)` 读取结果。`subtree` 为可选的骨名；例如 `j_te_r` 只混合右手及其后代。平移与缩放线性混合，旋转取四元数短弧。在下一次 `sample` 时从源动作重新求值，来回拖进度不会累积误差。过渡时长、权重与骨遮罩由宿主决定。

## 在动作中持械

头部和上身装备也可带自己的骨架。导出器按角色种族和部件编号查询 `extra_met.est`、`extra_top.est`；脸与发型分别查 `faceSkeletonTemplate.est`、`hairSkeletonTemplate.est`。`actorPartialRigs()` 返回实际参与蒙皮的路径，同一套局部姿态接口适用于这四类骨架。c0101 的发型 1 使用 `h0003`；上身 `e0100` 的根 `n_ex_top` 在基础骨架中没有同名关节，1300 格式 SKLB 的连接索引 22 指向 `j_sebo_c`。两者参考世界矩阵的最大分量差为 `3.05e-7`。播放器在缺少同名父链时，只接受连接索引有效且参考矩阵吻合的挂接。Vulkan 验证覆盖 `e0100` 上身和 `e0007` 头部：单独移动部件关节改变了最终像素，清除后恢复原图。此接口可以提交宿主的布料姿态，不会从权重猜测物理参数。

```js
await element.openEffect(weaponManifest, read);
await element.followWeaponHand(characterSklb, characterPap, {
  record: 1, bone: "n_buki_r", loop: true, grip: weaponToHand,
});
```

`grip` 是从武器模型空间到手骨局部空间的 4×4 矩阵，缺省为单位阵。武器顶点和粒子挂点共用 `hand × grip`，不把角色骨号当武器骨号。若只预览武器，可将 `grip` 设为起始手骨帧的逆，使武器从原预览位置开始、随后显示相对于起始帧的运动。这个预览选项不代表游戏的持握校正。

同一调用可指定 `overlay: {pap, record, weight}` 混合角色加法层，或指定 `weaponClip: {sklb, pap, record}` 同时播放武器自己的骨骼动作。后者按武器骨名提取局部 TRS，再把各根骨变换到手骨空间；内部关节仍保持其局部动画。

宿主也可以直接传入完整的武器局部姿态：

```js
const {names, rest} = await element.weaponSkeleton();
const pose = rest.slice();
const i = names.indexOf("n_hara");
if (i >= 0) pose[i * 10] += 0.05; // 局部 X 平移；T3、四元数 XYZW、S3
await element.setWeaponPose(pose);   // 转为宿主提供姿态
```

native 对应 `PapPose::bone_world`、`EffectPlayer::set_local_pose`、`set_weapon_grip` 和 `frame_with_host`。后者在初始发射和每个 60 Hz 模拟步之前提供绝对秒数。只在屏幕刷新时更新一次骨骼，会让同一显示帧里的多个粒子出生时刻拿到错误姿态。

角色在场景中的位移和转向可以独立于 PAP：`setActorPlacement([x,y,z], [qx,qy,qz,qw])` 提交绝对世界位置和旋转。新的骨姿态使用同一世界变换，双手武器和挂点也随之移动；镜头的自动取景不改变返回的挂点坐标。native 可在 `frame_with_host` 中逐步提交 `set_actor_placement`。这是宿主给出的根运动，不表示静态 PAP 包含战斗场景中的移动轨迹。

## 实资源对照

`effects/examples/verify_saw.rs` 以真实群星手锯的两份相同模拟为输入：一份维持绑定姿态，另一份逐帧平移、旋转并非均匀缩放 `n_hara`。从第 48 至 240 帧对照两面各三个环的中心和三条基向量，检查包数量、局部旋转、缩放以及是否进入后备路径：

```sh
cargo run --release --locked --manifest-path effects/Cargo.toml --example verify_saw -- saw.json objects
```

1,158 个环中心的最大欧氏误差为 `1.34e-7`，三条基向量的最大误差为 `7.26e-7`。这是对绘制包的仿射校验，不是游戏录像的像素匹配。REA 对该 AVFX（SHA-256 `77f9749b65324ae4327e8ae0c3826e9dcd134d178fca7b45013a83c23eae4d0b`）的 `p8 488 @ 0x1447c`（raw 基址 0，Evidence `ev_f1edec428bd0128d47d6526eca0d3a46da64070e6b30979f4d4b3255d2468fa7`）确认该 Bind 的 `bFTO=ff`、`bTSc=01` 和常量 `SpS`。测试的运动来自宿主，不是为手锯补造的 AVFX 曲线。
