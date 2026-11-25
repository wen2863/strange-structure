//! layout-pick.json 的绘制代码：面板、固定文字与控件矩形。
//!
//! 由 tools/layout2rs.py 从 JSON 生成，不要手改：改布局就改 JSON 再重新生成。
//! 坐标是面板内的相对坐标（480×270 设计），面板居中由调用方给 x/y。

use mod_api::DrawContext;

/// 面板尺寸。
pub const PANEL_W: f32 = 320.0;
pub const PANEL_H: f32 = 250.0;

/// 面板底色（取自 JSON）。
pub const PANEL_COLOR: u32 = 0xFF101010;

/// 右上角的当前外观名：右边缘、行高与颜色。
pub const NAME_RIGHT: f32 = 308.0;
pub const NAME_Y: f32 = 6.0;
pub const NAME_COLOR: u32 = 0xFFFFFF80;

/// 搜索栏（TextField 的矩形）。
pub const SEARCH: [f32; 4] = [10.0, 28.0, 300.0, 18.0];

/// 方块格：一页 COLUMNS × ROWS 格，格 TILE、步长 STEP，左上第一格在 GRID 的左上角。
pub const COLUMNS: usize = 8;
pub const ROWS: usize = 4;
pub const TILE: f32 = 28.0;
pub const STEP: f32 = 34.0;
pub const GRID: [f32; 4] = [27.0, 54.0, 266.0, 130.0];

/// 右侧滚动条的轨道（点击与拖动都在这个矩形里）。
pub const TRACK: [f32; 4] = [301.0, 54.0, 6.0, 130.0];

/// 底部三个按钮。
pub const BACK: [f32; 4] = [10.0, 214.0, 80.0, 22.0];
pub const PREV: [f32; 4] = [120.0, 214.0, 60.0, 22.0];
pub const NEXT: [f32; 4] = [188.0, 214.0, 60.0, 22.0];

/// 一个面板内的矩形放到屏幕上。
pub fn at(x: f32, y: f32, rect: [f32; 4]) -> [f32; 4] {
    [x + rect[0], y + rect[1], rect[2], rect[3]]
}

struct Label {
    text: &'static str,
    x: f32,
    y: f32,
    color: u32,
}

const LABELS: [Label; 1] = [Label {
    text: "外观 · 选取",
    x: 10.0,
    y: 6.0,
    color: 0xFFFFFFFF,
}];

/// 画面板底和固定文字；当前外观名由调用方按 NAME_RIGHT 右对齐后画。
pub fn draw(draw: &mut dyn DrawContext, x: f32, y: f32) {
    draw.fill([x, y, PANEL_W, PANEL_H], PANEL_COLOR);
    for label in &LABELS {
        draw.text(label.text, [x + label.x, y + label.y], label.color, false);
    }
}
