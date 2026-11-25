# 武器常驻特效

`effects/` 是独立的 native / WASM 播放模块。静态 `EquipmentViewer` 不需要它。浏览器从同一 SDK 的 `pkg/effects/` 按需导入；输入是一个武器部件的原始文件字节而非安装路径。

## 接入

宿主为武器创建 `<icexiv-equipment-canvas>`，插入文档后调用
`await element.openWeapon(effect, read)`；`read(path, sha256)` 返回校验过的 `Uint8Array`。
WebGPU 设备、模型解析及绘制运行在模块 Worker 的 OffscreenCanvas 上。
`await element.setEffectEnabled(true/false)` 在同一场景开关挂载，首次开启时读取特效文件。
需要直接开始播放时可用 `openEffect(effect, read)`。
`effect.main` 是十进制字符串形式的 packed model ID；`owner` 是实际加载的 MDL；
`mounts` 是实际加载的 AVFX 路径；`files` 是此场景完整的逻辑路径 → SHA-256。
包含宿主模型与材质、选定 IMC、AVFX、ATEX，以及挂点需要的 SKLB 等资源。
可用 `replayEffect()` 回到起点；移除元素停止动画并释放设备资源。`rendererror`
报告播放失败，`renderwarning` 报告可继续播放的绑定或 Aura 降级。

绘制核心在 `effects/scene/`，基于 `xiv-companion-render@1a7950a`，配合相同修订所用的数据加载器。特效场景保留武器原始坐标、ElementId、骨架和同一深度缓冲；主副手、角色与动作粒子由同一相机绘制。`setEffectEnabled` 开关常驻挂载；运行中的 C012 动作可以独立启停。
按 60 Hz 固定步长推进，单帧最多补 0.1 秒；页面隐藏或元素离开视口时暂停。主/副部件的文件与骨表分别处理。[骨骼动作与动态挂点](motion.md)说明如何用角色 PAP 驱动武器和常驻特效。

## 用实际游戏资产验证

各绘制路径的支持边界、群星手锯的遮挡对照和复现参数见[渲染支持范围](render-support.md)。

`cargo build --release --locked -j4 --manifest-path effects/Cargo.toml --examples`
构建原生导出器和 PNG 快照工具。以下资源只读自本机客户端：

```bash
mkdir -p /tmp/opencode/vfx-check
cd /tmp/opencode/vfx-check
# BIN 指向 renderer 仓库的 effects/target/release/examples；GAME 为客户端安装根。
echo 4298899957 | "$BIN/export" "$GAME/game" objects > lance.json
echo 4295098473 | "$BIN/export" "$GAME/game" objects > shield.json
"$BIN/snapshot" lance.json objects lance-frames
"$BIN/snapshot" shield.json objects shield-frames
```

快照使用与 companion 验收相同的相机角度和距离，比较关闭、0.8 秒、4 秒三个时刻。
`export` 对确认无挂载返回 `null`；解析失败返回 `error`；可挂载结果保留分组诊断。
`npm run build` 或 `nix build .#sdk` 同时提供静态模块和延迟加载的特效模块。
