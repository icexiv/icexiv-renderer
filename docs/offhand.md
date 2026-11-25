# 独立的左手武器

`setOffhandBundle` 把第二个武器部件装在当前角色的 `n_buki_l` 上。它接受与主武器相同的 `export` 清单；没有常驻特效的部件用 `export --owner`（仍需模型、材质和 SKLB）。例如群星手锯主部件为 `17180398476`，左手部件为 `4295037913`，后者的 `mounts` 为空。

```js
// 已用 openWeapon / setActorBundle 装载主武器和角色。
await viewer.setOffhandBundle(secondaryManifest, read, {grip});
await viewer.setOffhandVisible(false); // 保留装配、相机范围和正在运行的时钟
await viewer.setOffhandVisible(true);
await viewer.clearOffhand();           // 移除模型和对应的播放资源
```

`read(logicalPath, sha256)` 由宿主提供；`grip` 为 16 个有限数的列主序 4×4 矩阵，从副武器模型空间变到左手骨骼空间，默认单位阵。每次角色姿态变化都使用当前的 `world(n_buki_l) × grip` 更新副武器根、蒙皮调色板和 ElementId；不播放独立的副武器 PAP。该入口需要已经装载并有 `n_buki_l` 的角色，以及可准备蒙皮调色板的副武器。删除角色同时卸下副武器。替换清单失败会保留之前的副武器。

`await viewer.offhandDiagnostics()` 可在动作播放中单独读取副手绑定和 Aura 诊断；无副手时返回空数组。

Rust 对应接口在 `icexiv_effects::offhand`：`EffectPlayer::set_offhand_bundle(&OffhandManifest, read, OffhandOptions)`；本机已打开 SqPack 时可直接 `set_offhand(source, packed_id, resident, options)`。清单 `mounts: []` 对应 `resident=false`，不会尝试创建空的 AVFX。`offhand_elements()` 只返回此部件在当前姿态下的世界空间绑点。

主武器、角色和左手武器使用同一个实例列表做相机取景，绘入同一深度/颜色目标。左手常驻粒子在独立的资源表和 Document 时钟上运行，从装配时的零年龄开始；主手、左手与动作批次共用排序和混合阶段。`reset` 重放但不卸下部件。Aura 同时按拥有者的 GPU 实例和目标表面选择，因此两个部件可使用相同的逻辑 MDL 路径，亦不会把副武器的 Aura 混到角色或主手上。`triggerOffhand(scheduler, trigger)` 只触发左手常驻 Document；`triggerResident(owner, scheduler, trigger)` 仍选择主武器的 Document。

动作宿主可显式选择 `{origin:1,kind:2,id:…}` 作为 C010 对应部件的绑点来源。装有独立副武器时，这个选择只读取副武器；`kind:1` 仍读主武器。没有装副武器时，保留单部件查看器的约定：若宿主显式把当前单个挂载的武器当作副部件，`kind:2` 可读该挂载。两者的 ElementId 不拼成一张表，其他端点仍由动作宿主提供。运行时副手诊断带 `offhand <owner MDL>:` 前缀。

原生验收使用 `effects/tests/actor.rs` 和 `effects/tests/support/offhand.rs`。设置 `XIV_GAME_DIR` 指向本机游戏目录，再运行：

```sh
cargo test --manifest-path effects/Cargo.toml --test actor dressed_idle \
  -j4 -- --ignored --nocapture --test-threads=1
```

`OFFHAND_SNAPSHOT_DIR` 可指定保存三张 RGBA 图像的目录：固定取景的左手隐藏、显示，以及同一 MDL 装在两手并有两个常驻特效和动作特效的合成。NVIDIA Vulkan 实测中，`256×256` 画面单独切换 `w5081/b0001` 的左手可见性改变了 3,339 个像素；替换读取失败后的画面保持不变。两手改用 `w5004/b0008` 后，只移动左手时副武器各 ElementId 的位移与该骨的世界位移相符（容差 `1e-4`），叠加 `cmcn_shoot1c.avfx` 后重置重播逐字节一致。这个对照验证预览器的绘制和回放，不推断客户端的实际握持偏移或技能触发条件。
