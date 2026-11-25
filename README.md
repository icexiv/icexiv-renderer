# icexiv-renderer

FFXIV 模型、骨骼动作和特效预览器，提供 Rust 与 WebGPU 接口。输入为从 SqPack 提取的逻辑文件：装备 MDL（Win32 `0x01000005` 或 `0x01000006`）、材质和纹理、配套的 SKLB／PAP，以及 AVFX 和它引用的资源。

预览显示 LOD 0 的 normal/glass 网格，自动居中，以相机相对光源显示绑定姿势。传入配套 MTRL 和 TEX 后，每个材质槽可使用索引图与 Dawntrail 色表、或漫反射贴图绘制装备配色。只传 MDL 时显示中性材质。轨道相机与灯光基于 `xiv-companion` (`1a7950a`) 的 `renderer/model/material/uniforms.rs`。支持的采样器和外观限制见[装备模型预览](docs/README.md)，按变体绑定材质见[宿主集成](docs/integration.md)。

## 开发与构建

`nix develop` 提供 Rust（含 WASM 目标）、匹配的 wasm-bindgen CLI、Node.js 和 Notist。`nix build .#native` 构建 Rust 库，`.#sdk` 构建可分发的 Web SDK，`.#action-import` 构建动作资源导出器，`.#site` 构建文档站。使用 `nix flake check` 检查构建产物。在开发环境中运行 `notist build` 可将文档写入 `notist-output/icexiv-renderer/`。

## Web SDK

[`EquipmentViewer` 与 `SkeletonClip`](docs/motion.md) 支持装备蒙皮和独立骨姿态采样。[武器常驻特效](docs/weapon-effects.md)由 `effects/` 模块按需提供；在同一个场景中绘制武器与特效，可随角色的持械骨更新挂点。

[从 Action ID 播放动作与特效](docs/action.md)给出飞石（119）的资源导出、浏览器示例和 SDK 调用方式。

需要带 `wasm32-unknown-unknown` 标准库的 Rust 工具链、Node/npm 和与 Cargo.lock 匹配的 `wasm-bindgen-cli 0.2.129`。

```sh
npm run build
```

输出 `pkg/icexiv_renderer.js`、`pkg/icexiv_renderer_bg.wasm` 与 TypeScript 声明。将 `pkg/` 放到静态站点，保留 JS 和 WASM 的相对位置。构建后也可以 `npm pack`，在宿主项目安装本地 tarball，以 `@icexiv/renderer` 导入。WASM 文件使用 `application/wasm`。示例 `examples/viewer.html` 可通过 localhost 或 HTTPS 的静态站点打开，选择本地文件并拖动旋转。

```js
import init, { EquipmentViewer } from "./pkg/icexiv_renderer.js";

await init();
const viewer = await EquipmentViewer.create(canvas); // canvas.width/height 是像素大小
viewer.setModel(mdlBytes); // Uint8Array
viewer.draw(0.35, 0.1);   // yaw、pitch，弧度。调整大小或角度后再 draw
// 替换装备时可复用 viewer。页面卸载时：
viewer.free();
```

页面应捕获初始化/加载错误：浏览器需支持 WebGPU，MDL 解析错误会抛出 JavaScript Error。SDK 不自建帧循环。

## CI 与发行

PR 和 `main` 构建执行锁定的 `nix flake check`（native、WASM SDK、Notist），并保存 Web SDK 工件。推送到 `main` 会更新滚动预发布 `dev-build`；推送 `v*` 标签发布对应版本。Release 提供 `icexiv-renderer-web.tgz` 和 `SHA256SUMS`；部署方固定标签及 SDK 摘要，native 宿主可固定同一 Git 修订。SDK 不包含游戏资源。

## Native 接入

`EquipmentModel::from_mdl(&bytes)?` 解析模型；`Renderer::new(device, queue, format)` 接收宿主的 wgpu 设备。调用 `renderer.upload(&model)` 得到可复用的 `GpuModel`。`renderer.draw(&target_view, [width, height], &gpu_model, yaw, pitch)?` 清屏并提交，宿主负责呈现。目标为同一设备、同一格式的单采样颜色附件。

完整的离屏示例会校验输出有可见像素并写入透明 PNG：

```sh
cargo run --locked -j4 --example snapshot -- equipment.mdl preview.png
```

可在最后追加一个资源目录，其中放置该变体的 MTRL 和 TEX（按唯一文件名匹配）。网页示例支持一次选择 MDL 与配套文件。正式集成应按完整逻辑路径定位纹理，避免不同目录下的同名文件混用。
