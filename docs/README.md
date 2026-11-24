# 装备模型预览

输入为从 SqPack 提取的 Win32 `0x01000005` 或 `0x01000006` 逻辑 MDL。渲染器将 LOD 0 的 normal 和 glass 网格居中，以绑定姿势绘制。没有这两类网格时，可预览水面、光束、雾或替换网格。材质槽只列出参与绘制的网格所引用的材质，按首次使用排序。所有位置重合的模型返回 `NoVisibleExtent`。

水面使用 `g_WaterDeepColor`、光束使用 `g_Color` 作为静态不透明近似；不模拟折射、光照散射或动画。角色材质的每一行 diffuse 相同时可直接使用表内颜色，无需索引纹理。

材质颜色取自 MTRL 声明的采样器：`g_SamplerIndex` 配合 Dawntrail 色表，用索引图的红通道选择行对，绿通道作为偶数行权重，混合两行的 diffuse；`g_SamplerDiffuse` 或 `g_SamplerColorMap0` 则使用 sRGB 基色图。未绑定资源的槽显示为中性灰色。此颜色预览按不透明表面绘制，不包含染色、法线贴图、遮罩、金属反射和透明混合。
