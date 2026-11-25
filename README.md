# 异构 / The Strange Structure

一个 Horizon 26.3 的原生（Rust）模组：一个不可破坏、可自由浮空、可缩放旋转、能吸附并转化物品的方块。

## 三档？不，是一颗方块

| 能力 | 说明 |
| --- | --- |
| 尺寸 | 0.2 ~ 5.0 格，按 0.2 步进（单一立方值） |
| 状态 | 刚体（有碰撞）/ 虚体（可穿过） |
| 外观 | **本体**（自带 64×64 皮肤）或游戏里任意方块：点「选取」进二级页，带搜索栏，每页 32 格，滚轮 / 拖动条 / 上一页 / 下一页翻。箱子（含陷阱/末影/铜）、铜傀儡雕像、旗帜、潜影盒、头颅、饰纹陶罐、潮涌核心、钟这些由方块实体渲染的方块改用物品模型显示 |
| 生成 | 每 100 个区块 1 个，落在地表 |
| 创造模式 | 独立的「异构」标签页（第 2 页），放本体方块、奇异宝石、奇异碎片 |
| 吸附 | CTRL+左键把手持物品塞进链头，可无限叠加；空手 CTRL+左键从链尾取回最老的一件。方块放大后点到组成它的隐形填充块也算 |
| 破坏 | 创造模式下打掉放大方块的任意一块隐形填充块，整个方块连同其余填充块一起消失；打本体同理 |
| 转化 | 钻石 5 秒 → 奇异宝石；下界合金碎片 8 秒 → 奇异碎片 |
| 光球 | 物品转化的那几秒里，方块与物品之间会连出一串紫色光球；转化完成即消失 |
| 绘制 | 每个面一张 32×32 画板，16 色调色板、三档笔尖（细 1×1 / 中 3×3 / 粗 5×5 格）加橡皮，画完跟着方块一起存盘 |

## 操作

- **CTRL+右键**：打开设置屏（状态 / 大小 / 外观）
- 外观行显示当前外观名：「选取」进外观页（8 列×4 行共 32 格，列出游戏里所有方块），顶部搜索栏按名字过滤（空格清不掉就 CTRL+A、退格），右侧拖动条可以直接拖到任意位置，滚轮按滚动量走、Shift 一次 4 行，也有上一页/下一页，打开时自动翻到当前外观所在页；点一格即选中并回到设置屏；「绘制」进画板页
- 画板页左侧是当前面的 32×32 画板，中间竖排六个面按钮，右侧竖排三档画笔加橡皮、最右两列 16 色；按住左键拖着画；「清空」清掉当前面，「返回」回设置屏
- 设置屏的「完成」关屏
- **CTRL+空格**：抓取准星上的方块，再按一次放下（放大后点到隐形填充块也能抓）
- 抓取中：滚轮调距离，方向键每次旋转 10°
- **CTRL+左键（手持物品）**：把物品塞进方块的链头（无限叠加，新物品在最前）
- **CTRL+左键（空手）**：从链尾取出最老的一件物品，直接落到你脚下

## 改界面

三个页面都不是手写坐标，而是各自从一份布局 JSON 生成的：

| 页面 | 布局 JSON | 生成的模块 |
| --- | --- | --- |
| 设置（状态/大小/外观） | `layout-gui.json` | `src/gui_layout.rs` |
| 外观 · 选取 | `layout-pick.json` | `src/gui_pick.rs` |
| 外观 · 绘制 | `layout-paint.json` | `src/gui_paint.rs` |

改法：

1. 浏览器打开 `tools/gui-editor.html`，用「打开 JSON」载入对应文件，拖动/缩放控件，右侧改文字与颜色；
2. 「保存 JSON」覆盖回同名文件；
3. 跑转换器（需要 Python 3），再重新构建：

   ```
   python tools/layout2rs.py layout-gui.json src/gui_layout.rs
   python tools/layout2rs.py layout-pick.json src/gui_pick.rs
   python tools/layout2rs.py layout-paint.json src/gui_paint.rs
   ```

转换器按元素类型认页：有 `icon` 的是选取页、有 `board` 的是绘制页、其余按设置页处理。生成的模块里是面板尺寸、颜色、`LABELS` 与各控件矩形（选取页的格阵 `GRID/COLUMNS/ROWS/TILE/STEP`、搜索栏 `SEARCH`、拖动条 `TRACK`，绘制页的 `CANVAS`、六面 `FACES`、画笔 `BRUSHES`、色块 `SWATCHES` 三组矩形数组），`lib.rs` 只按这些常量画面板、建控件。设置页的数值列会自动躲开同一行的按钮（留 8 px），外观名过长会截断成 `..`。

## 构建

```
$env:HORIZON_SDK_BASE=D:\Downloads\horizon-sdk
cargo horizon-mod build                 # 打包成 .hmod
horizon-launcher native-build . --instance horizon --trust-native-code
```

## 目录

- `src/lib.rs`：钩子、屏幕、载荷，以及移动 / 吸附 / 生成的服务器侧
- `src/units.rs`：尺寸（十分之一格、偶数步进）、填充块的切分键、方块实体的简单读取
- `src/paint.rs`：六面绘制、调色板与笔刷、客户端为每段像素画的薄板
- `src/chain.rs`：链上的物品、转化计时、方块实体里存它们的 Compound
- `src/render.rs`：本体皮肤、光球与环绕轨迹、选取页要用的物品图标
- `src/gui_layout.rs`、`src/gui_pick.rs`、`src/gui_paint.rs`：三页的布局常量（面板、文字、按钮与格阵矩形），分别由 `layout-gui.json`、`layout-pick.json`、`layout-paint.json` 经 `tools/layout2rs.py` 生成，不要手改
- `layout-*.json` / `tools/layout2rs.py` / `tools/gui-editor.html`：三页布局原件、JSON→Rust 转换器、可视化布局编辑器（浏览器打开）
- `data/strange_structure/horizon/block/`：两个方块的声明（本体 + 隐形填充块）
- `data/strange_structure/horizon/item/`：两个新物品
- `assets/strange_structure/`：模型、语言文件；贴图为 `textures/block/strange_skin.png`（本体皮肤，64×64，走方块图集）+ `textures/block/paint_white.png`（画板底色，16×16）+ `textures/item/strange_gem.png`、`textures/item/strange_shard.png`（各 64×64）+ `textures/entity/orb.png`（转化光球）
- `data/strange_structure/horizon/creative_tab/strange_structure.json`：独立「异构」创造标签页
- `设计文档.md`：设计、API 考证与实测数据

## 许可

MIT，见 `LICENSE`。