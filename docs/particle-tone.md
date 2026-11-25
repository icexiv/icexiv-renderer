# `bATM`：粒子片元的亮度倍率

`Ptcl.bATM` 在 Apricot 前向像素着色器中控制一个**依赖场景输入的 RGB 倍率**。它既不是后处理 bloom 开关，也不是给单个粒子套一次整屏 tone mapper。倍率可能大于、等于或小于 1；仅凭手锯资产里的 `bATM=1` 无法预测锯刃会亮多少。

## 从 AVFX 到像素常量

本机 `ffxiv_dx11.exe` SHA-256：`52042b60631cf2c48851b3aaee36e9ec4fedd6d7f2c68f0e9a8c3befb0af9f98`。以下为静态 VA，映像基址 `0x140000000`。REA/Rizin 指令与表项取证：`ev_f016c1e53f187a135d974b3f4143cdea55f7d9d8ae95daba158d47ab2e5fcce1`。

| 位置 | 可复核的数据流 |
| --- | --- |
| `1403a721a..725b`、`1403a8c6c..8c89` | 定义 `+0x2a` 初始为零。标签 `0x6241544d` 将负载首字节 **bit 0** 写到定义的 bit 4，保留其它位。紧邻的 `bAFg` 写 bit 5。固定版数据解析器 `read_bool` 按非零读取，故仅对标准 `0/1` 负载等价。 |
| `1403e1ff0..205d` | 从实例 `+0x368` 取定义指针；`(def[0x2a] & 0x10) << 9` 合入实例 `+0x358` 的 bit 13。 |
| `1403e9695..96d3`、`14037f43d..444` | 一个实际绘制调用把该状态字作为第五实参传给 `14037f3b0`；更新批次状态时保存到 `+0x38`。 |
| `14037fb1f..fbb9` | `bt ecx,0xd` 将首个 float4 的 W 置 `1.0` 或 `0.0`；bit 14/15 另占 Y/Z。调用 `140388e10`，参数索引 `0x26`、长度两个 float4。相同 bit 测试亦见 `14037fd4d`、`14037ffea`、`140380baf`、`140381145`。 |
| `140384d88..4dfd`、`140388e53..8e7e` | 初始化表步长 16；`0x1421611f0 + 0x26*16 = 0x142161450` 的记录是 `g_PS_InstanceExtraParameters`、数量 `2`。更新使用同一索引并复制 `2*16` 字节。**`0x26` 是客户端表索引，不是硬编码的 DXBC `cb` 槽。** |

## 方程与覆盖

从安装目录 `shader/sm5/shpk/apricot_*.shpk` 读取程序表，以各 DXBC 的 `RDEF` 名称解析资源槽，检查 `SHEX` 指令与源/目的分量。记 `I = g_PS_InstanceExtraParameters[0].w`、`T = g_ToneMapParameter[0]`：

```text
R = sample(g_ToneMapSampler, uv=(0.5, 0.5)).r
K = rsqrt(mad(T.x, R - 1, 1))
gain = mad(I, K - 1, 1)
```

这是标量指令顺序；没有从指令中读到给 `rsqrt` 添 epsilon 或饱和钳位。在正常有限正输入域，关闭时 `gain=1`，开启时 `gain=K`。另有**不受 `bATM` 控制**的 `T.w` 乘色。Shape PS8、LightModel PS1、Model PS26、Powder PS1 中，主颜色目标的 RGB 先乘 `T.w` 再乘 `gain`；该倍率不乘 alpha。亮度会影响这些程序的第二渲染目标，不可据此断言所有输出通道都不变。

| 包 / 程序索引 | DXBC 在 SHPK 内偏移 | DXBC SHA-256 | `InstanceExtra` / `ToneMapParameter` |
| --- | ---: | --- | --- |
| Shape PS8 | `0x296088` | `6f371f4b36697057d24590c42dd1740c1f5ac2112625389686b357ce31b37bff` | `cb0` / `cb2` |
| LightModel PS1 | `0x3e0054` | `7fe2bdc97749feeae9cf9040a068745ac363179e76a12149580d0669ea765c71` | `cb0` / `cb3` |
| Model PS26 | `0x45ec98` | `6a1590dd580992e9e7137f2d7f0d9ac96e65c88b82f4a32b8a89fac31e6ff356` | `cb1` / `cb4` |
| Powder PS1 | `0x16014` | `04a9ef9ce0939885fe11708f5969fc6a538b0cc08325d04ab5b4effeb78fe36f` | `cb0` / `cb2` |

例如 LightModel PS1：SHEX DWORD 240 为 `T.w` 的 RGB 乘法；265 的 `sample r0.w, …, t1.yzwx, s1` 将纹理 **X**（重排的第四项）写入 W；279/286/296/301/308 对应 `add/mad/rsq/add/mad`；318 乘 RGB，325 写 `o0`。可用 `icexiv-data extract` 从自己的安装中导出该包，在表中偏移处按 DXBC 头 `+24` 的小端 u32 长度截取，再用 `vkd3d-compiler -x dxbc-tpf -b d3d-asm` 核对；无需分发客户端字节。

对十个非 dummy 包按表项核对，以下 **18,312** 个像素程序都含上述中心红通道采样和同寄存器的五步倍率计算：shape 3,538、lightmodel 5,459、model 5,930、powder 48、model_morph 168、fogModel 1,025、decal 1,061、decal_ring 1,083。另八个程序（gbuffer_decal 7、gbuffer_decal_ring 1）没有完整资源组合，不能套用此阶段。计数只证明标量计算，不证明每个变体都以相同顺序合成：例如 Shape PS810 后续有饱和乘法，LightModel PS2666 有额外的颜色插值。未从这次静态检查确定手锯在特定帧绑定的 PS 索引或当帧输入值。

## 渲染器接入

`effects/scene` 将 `(T.x, T.w, R)` 随相机 uniform 上传。`vfx.wesl::compose_color` 在纹理、Fresnel 与 alpha 淘汰之后、混合模式的钳位与硬件混合之前运行上式。`bATM` 占材质分组标志 bit 18；其余组键相同且一个开、一个关的实例不会错误合批。四边形、折线和网格在批次排序前分别由 `update_with_tone` / `update_mesh_with_tone` 读取与输入包逐项对应的开关。旧 `update` / `update_mesh` 相当于全关。独立深度投影 decal、G-buffer、宿主材质/Aura、整屏 bloom 和后处理都不在该 GPU 路径中。

| 宿主 | 调用 |
| --- | --- |
| Rust 播放器 | `player.set_particle_tone(t_x, t_w, center_red)?` |
| Web 组件 | `await viewer.setParticleTone(tX, tW, centerRed)` |
| 直接使用 scene | `ModelRenderOptions { particle_tone: [t_x, t_w, center_red], ..Default::default() }`，随该帧提交 |

播放器保留最近一次输入，直到宿主更新；预览默认 `[0, 1, 1]` 是中性策略，不是客户端默认值。播放器拒绝非有限输入以及非正或非有限的 `rsqrt` 参数；直接使用 scene 的调用者应提供同一有效域。指定 `T.w != 1` 会改变**包括 `bATM=0` 在内**的前向粒子。SDK 不生成客户端的曝光历史或色调纹理；`R` 应由宿主从对应帧提供。未核对复杂变体的其余合成、客户端第二渲染目标或真实场景的逐像素结果。

常驻附件和动作的包各自携带定义开关。Quad 有 `particle_index`；数据依赖的模型包没有该索引。同一几何可由开关相反的两个定义引用，因此 `ParticleMaterials` 在私有播放副本上用只透传至模型包的 `DOTy` 槽携带定义索引，并在上传前恢复作者的原始值，不按模型 ID 反推定义。升级数据依赖时应复核该槽仍只被透传。

固定数据依赖生成的原始解析诊断仍可能说前向绘制缺少色调阶段；这条文本不检查本地 scene 的着色器或宿主是否已提供曝光输入。

**实测：** `effects/tests/particle_tone.rs` 在 NVIDIA GeForce RTX 4070 Ti SUPER / Vulkan 上提交两份 Quad 和两份共享同一模型的 LightModel，使用 opacity 写入模式，颜色相同，alpha 均为 `0.375`。读取 bloom 之前的 `RGBA16Float`，分别检查中性输入、仅 `T.w=2` 及 `T.x=0.5,T.w=2,R=7`。四个粒子各覆盖 256 像素：开启侧由 `[0.5,1,4]` 降到 `[0.25,0.5,2]`，关闭侧保持 `[0.5,1,4]`；三次读回的 alpha 相同，其余 3,072 个背景像素未变。此对照验证受控场景输入，不是游戏截图比对。

```sh
XIV_ALLOW_GPU_TESTS=1 cargo test --manifest-path effects/Cargo.toml \
  --features gpu-tests --test particle_tone -j4 -- --ignored --nocapture --test-threads=1
```
