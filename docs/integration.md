# 宿主集成

## 浏览器

交互预览可导入 `@icexiv/renderer/component`，将 `<icexiv-equipment-canvas>` 放入页面后调用 `const viewer = await element.open()`。用 `viewer.setModel`、`viewer.setMaterial` 装配完成后调用 `element.show()`。组件负责高 DPI 尺寸、按需重绘、轨道相机和缩放，移出 DOM 时释放 viewer。`rendererror` 事件的 `detail` 为绘制错误。直接使用低层 Canvas API 时由宿主管理这些生命周期。

浏览器需要 WebGPU 和安全上下文。静态部署时将 SDK 的 `pkg/` 整体复制到站点，保留 JS 与 WASM 的相对位置。

```js
import init, { EquipmentViewer } from "./pkg/icexiv_renderer.js";

await init();
const viewer = await EquipmentViewer.create(canvas);
viewer.setModel(mdlBytes);  // Uint8Array
viewer.draw(0.35, 0.1);     // yaw、pitch（弧度）
```

调整 canvas 像素尺寸、旋转视角或替换模型后调用 `draw`。组件卸载时调用 `viewer.free()`。

### 应用游戏材质

`materialNames()` 按槽返回 MDL 引用。宿主用装备变体与资源清单定位 MTRL，传入的资源均为从 SqPack 提取的文件字节。下面的 `read(path)`（返回 `Uint8Array`）和 `manifest` 由宿主提供：

```js
import { baseColorTexture, materialVariant } from "./pkg/icexiv_renderer.js";

const variant = materialVariant(imcBytes, subset, partBit);
for (const [slot, ref] of viewer.materialNames().entries()) {
  const path = manifest.materials[variant][ref]; // 可包含独立的身体材质路径
  const mtrl = await read(path);
  const texturePath = baseColorTexture(mtrl);
  const tex = texturePath ? await read(texturePath) : new Uint8Array();
  viewer.setMaterial(slot, mtrl, tex);
}
viewer.draw(0.35, 0.1);
```

`partBit` 是 IMC 的部件位号，`subset = 0` 表示默认记录。`materialVariant` 只返回材质目录选择值，不应用 IMC 的部件可见性。TEX 使用 MTRL 采样器引用的逻辑路径；不支持的颜色来源会返回错误，失败的更新保留原槽绑定。

宿主也可调用 `materialPaths(modelPath, ref, variant)` 获取有序候选：相对文件名先尝试所选变体，再查无版本目录，最后尝试 `v0001`。完整的游戏路径直接使用。仅在确认文件不存在时尝试下一项，记录实际使用的路径；回退材质不保证与游戏中的配色一致。

## Native

`EquipmentModel::from_mdl` 接受字节切片。使用宿主的 `wgpu::Device`、`Queue` 和颜色格式创建 `Renderer`，调用 `upload` 得到可跨帧复用的 `GpuModel`。`draw` 接收同一设备上的单采样颜色附件及其像素尺寸；宿主负责呈现。

`gpu_model.material_names()`、`base_color_texture` 和 `renderer.set_material` 对应网页端的三个材质接口。`base_color_texture` 为 `None` 时传空纹理切片。`examples/snapshot.rs` 展示了材质加载、离屏渲染、像素回读及 PNG 编码。
