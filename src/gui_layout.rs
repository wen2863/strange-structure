//! layout-gui.json 的绘制代码：面板、固定文字与控件矩形。
//!
//! 由 tools/layout2rs.py 从 JSON 生成，不要手改：改布局就改 JSON 再重新生成。
//! 坐标是面板内的相对坐标（480×270 设计），面板居中由调用方给 x/y。

use mod_api::DrawContext;

/// 面板尺寸。
pub const PANEL_W: f32 = 360.0;
pub const PANEL_H: f32 = 180.0;

/// 面板底色（取自 JSON）。
pub const PANEL_COLOR: u32 = 0xFF101010;

pub const VALUE_COLOR: u32 = 0xFFFFE97F;

/// 数值列的右边缘（面板内 x）：各行共用一个，视觉上才成一列。
pub const VALUE_RIGHT: f32 = 159.0;
pub const SIZE_ROW_Y: f32 = 72.0; // JSON 右边缘 167.0，夹到按钮左侧 8px
pub const TEX_ROW_Y: f32 = 106.0; // JSON 右边缘 147.0

/// 控件矩形 [x, y, w, h]，交给 Widgets 建按钮。
pub const SOLID_ON: [f32; 4] = [112.0, 32.0, 106.0, 22.0];
pub const SOLID_OFF: [f32; 4] = [226.0, 32.0, 106.0, 22.0];
pub const MINUS: [f32; 4] = [180.0, 68.0, 22.0, 22.0];
pub const PLUS: [f32; 4] = [214.0, 68.0, 22.0, 22.0];
pub const TO_PICK: [f32; 4] = [167.0, 102.0, 58.0, 22.0];
pub const TO_PAINT: [f32; 4] = [240.0, 102.0, 58.0, 22.0];
pub const DONE: [f32; 4] = [120.0, 142.0, 120.0, 22.0];

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

const LABELS: [Label; 4] = [
    Label {
        text: "异构",
        x: 12.0,
        y: 8.0,
        color: 0xFFFFFFFF,
    },
    Label {
        text: "状态",
        x: 16.0,
        y: 38.0,
        color: 0xFFFFFFFF,
    },
    Label {
        text: "大小",
        x: 16.0,
        y: 72.0,
        color: 0xFFFFFFFF,
    },
    Label {
        text: "外观",
        x: 16.0,
        y: 106.0,
        color: 0xFFFFFFFF,
    },
];

/// 画面板底和固定文字；数值行由调用方按 VALUE_RIGHT 右对齐后画。
pub fn draw(draw: &mut dyn DrawContext, x: f32, y: f32) {
    draw.fill([x, y, PANEL_W, PANEL_H], PANEL_COLOR);
    for label in &LABELS {
        draw.text(label.text, [x + label.x, y + label.y], label.color, false);
    }
}
