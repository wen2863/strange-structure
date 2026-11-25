# -*- coding: utf-8 -*-
"""layout2rs: 把 Horizon GUI 编辑器导出的 layout JSON 变成可编译的 Rust 绘制模块.

用法: python layout2rs.py <layout.json> <out.rs>

按 JSON 里的元素自动判断是哪一页:
  * 有 icon 元素  -> 外观选取页：搜索栏、方块格阵、可拖动滚动条、返回/上一页/下一页；
  * 有 board 元素 -> 画板页：画板、六面、三档画笔、橡皮、16 色、返回/清空；
  * 其余          -> 设置页：面板、固定文字、数值槽、按钮。

约定（和 native 屏的绘制思路一致）:
  * 坐标以 JSON 里的面板元素为原点，面板本身在屏幕居中，由调用方传入 x/y；
  * 左对齐的 label 是固定文字，直接 draw.text（各带自己的颜色）；
  * 右对齐的 label 是数值槽（大小、外观名这类运行时变的文字），
    只取它的右边缘，数值统一右对齐到最右的那个槽（JSON 里手拖的框常常不齐）；
  * button / icon / swatch 按 JSON 的顺序与矩形逐个导出，不假设等距——手拖过的位置就是它；
  * 名字叫 search 的 panel 是搜索栏的矩形，名字叫 track 的 panel 是滚动条的轨道。
"""
import json
import sys


def color(hexs):
    h = hexs.lstrip("#")
    if len(h) == 6:
        h = "FF" + h
    return "0x" + h.upper()


def f(v):
    return "%.1f" % float(v)


ROW_NAMES = {"大小": "SIZE", "外观": "TEX", "状态": "STATE", "速度": "SPEED", "范围": "RANGE"}


def ident(name, fallback):
    if not name:
        return fallback
    return "".join(c if c.isalnum() or c == "_" else "_" for c in name).upper()


def detect(els):
    types = {e["type"] for e in els}
    if "icon" in types:
        return "pick"
    if "board" in types:
        return "paint"
    return "main"


def rect(e, ox, oy):
    return "[" + f(e["x"] - ox) + ", " + f(e["y"] - oy) + ", " + f(e["w"]) + ", " + f(e["h"]) + "]"


class Out:
    def __init__(self, source):
        self.lines = []
        head = source.replace(chr(92), "/").split("/")[-1]
        self.add("//! " + head + " 的绘制代码：面板、固定文字与控件矩形。")
        self.add("//!")
        self.add("//! 由 tools/layout2rs.py 从 JSON 生成，不要手改：改布局就改 JSON 再重新生成。")
        self.add("//! 坐标是面板内的相对坐标（480×270 设计），面板居中由调用方给 x/y。")
        self.add("")
        self.add("use mod_api::DrawContext;")
        self.add("")

    def add(self, line=""):
        self.lines.append(line)

    def text(self):
        return "\n".join(self.lines) + "\n"


def emit_common(out, panel, pw, ph):
    out.add("/// 面板尺寸。")
    out.add("pub const PANEL_W: f32 = " + f(pw) + ";")
    out.add("pub const PANEL_H: f32 = " + f(ph) + ";")
    out.add("")
    out.add("/// 面板底色（取自 JSON）。")
    out.add("pub const PANEL_COLOR: u32 = " + color(panel["color"] if panel else "#101010") + ";")
    out.add("")


def emit_labels(out, labels, ox, oy):
    out.add("struct Label {")
    out.add("    text: &'static str,")
    out.add("    x: f32,")
    out.add("    y: f32,")
    out.add("    color: u32,")
    out.add("}")
    out.add("")
    out.add("const LABELS: [Label; %d] = [" % len(labels))
    for l in labels:
        out.add("    Label { text: " + json.dumps(l["text"], ensure_ascii=False) +
                ", x: " + f(l["x"] - ox) + ", y: " + f(l["y"] - oy) +
                ", color: " + color(l.get("textColor", "#FFFFFF")) + " },")
    out.add("];")
    out.add("")


def emit_at(out):
    out.add("/// 一个面板内的矩形放到屏幕上。")
    out.add("pub fn at(x: f32, y: f32, rect: [f32; 4]) -> [f32; 4] {")
    out.add("    [x + rect[0], y + rect[1], rect[2], rect[3]]")
    out.add("}")
    out.add("")


def emit_draw(out, extra=""):
    out.add("/// 画面板底和固定文字" + extra + "。")
    out.add("pub fn draw(draw: &mut dyn DrawContext, x: f32, y: f32) {")
    out.add("    draw.fill([x, y, PANEL_W, PANEL_H], PANEL_COLOR);")
    out.add("    for label in &LABELS {")
    out.add("        draw.text(label.text, [x + label.x, y + label.y], label.color, false);")
    out.add("    }")
    out.add("}")


def emit_main(data, out, ox, oy, pw, ph, panel):
    buttons = [e for e in data["elements"] if e["type"] == "button"]
    labels = [e for e in data["elements"] if e["type"] == "label"]
    static = [e for e in labels if e.get("align") != "right"]
    slots = [e for e in labels if e.get("align") == "right"]

    slot_rows = []
    for s in sorted(slots, key=lambda e: e["y"]):
        same_row = [l for l in static if abs(l["y"] - s["y"]) <= 8]
        row = min(same_row, key=lambda l: abs(l["y"] - s["y"])) if same_row else s
        name = ROW_NAMES.get(row["text"], "ROW%d" % len(slot_rows))
        slot_rows.append((name, s["x"] + s["w"] - ox, row["y"] - oy))

    # 数值列不能顶到右边的按钮：按同一行最近的按钮留 8px 间距
    def gap_for(y):
        row_buttons = [b for b in buttons if b["y"] - oy - 4 <= y <= b["y"] - oy + b["h"] + 4]
        starts = [b["x"] - ox for b in row_buttons if b["x"] - ox > 0]
        return min(starts) - 8.0 if starts else None

    right = max([r[1] for r in slot_rows], default=0.0)
    gaps = [gap_for(r[2]) for r in slot_rows if gap_for(r[2]) is not None]
    value_right = min([right] + gaps) if gaps else right

    emit_common(out, panel, pw, ph)
    out.add("pub const VALUE_COLOR: u32 = " + color(slots[0]["textColor"] if slots else "#FFFFFF") + ";")
    if slot_rows:
        out.add("")
        out.add("/// 数值列的右边缘（面板内 x）：各行共用一个，视觉上才成一列。")
        out.add("pub const VALUE_RIGHT: f32 = " + f(value_right) + ";")
        for name, r, y in slot_rows:
            note = f(r)
            if value_right < r:
                note += "，夹到按钮左侧 8px"
            out.add("pub const " + name + "_ROW_Y: f32 = " + f(y) + "; // JSON 右边缘 " + note)
    out.add("")
    out.add("/// 控件矩形 [x, y, w, h]，交给 Widgets 建按钮。")
    for i, b in enumerate(buttons):
        out.add("pub const " + ident(b.get("name"), "BUTTON_%d" % i) + ": [f32; 4] = " + rect(b, ox, oy) + ";")
    out.add("")
    emit_at(out)
    emit_labels(out, static, ox, oy)
    emit_draw(out, "；数值行由调用方按 VALUE_RIGHT 右对齐后画")


def emit_pick(data, out, ox, oy, pw, ph, panel):
    els = data["elements"]
    icons = sorted([e for e in els if e["type"] == "icon"], key=lambda e: (e["y"], e["x"]))
    first = icons[0]
    row = sorted([e for e in icons if abs(e["y"] - first["y"]) < 1.0], key=lambda e: e["x"])
    columns = len(row)
    step = (row[1]["x"] - row[0]["x"]) if columns > 1 else first["w"]
    tile = first["w"]
    rows = len(icons) // columns
    gap = step - tile
    grid_w = columns * step - gap
    grid_h = rows * step - gap
    left = first["x"] - ox
    top = first["y"] - oy

    labels = [e for e in els if e["type"] == "label"]
    static = [e for e in labels if e.get("align") != "right"]
    name_slot = next((e for e in labels if e.get("align") == "right"), None)

    buttons = [e for e in els if e["type"] == "button"]
    by_name = {b.get("name"): b for b in buttons if b.get("name")}
    by_text = {b.get("text"): b for b in buttons}
    back = by_name.get("back") or by_text.get("返回") or (buttons[0] if buttons else None)
    prev = by_name.get("prev") or by_text.get("上一页")
    nxt = by_name.get("next") or by_text.get("下一页")

    search = next((e for e in els if e["type"] == "panel" and e.get("name") == "search"), None)
    track = next((e for e in els if e["type"] == "panel" and e.get("name") == "track"), None)

    emit_common(out, panel, pw, ph)
    if name_slot is not None:
        out.add("/// 右上角的当前外观名：右边缘、行高与颜色。")
        out.add("pub const NAME_RIGHT: f32 = " + f(name_slot["x"] + name_slot["w"] - ox) + ";")
        out.add("pub const NAME_Y: f32 = " + f(name_slot["y"] - oy) + ";")
        out.add("pub const NAME_COLOR: u32 = " + color(name_slot.get("textColor", "#FFFFFF")) + ";")
        out.add("")
    if search is not None:
        out.add("/// 搜索栏（TextField 的矩形）。")
        out.add("pub const SEARCH: [f32; 4] = " + rect(search, ox, oy) + ";")
        out.add("")
    out.add("/// 方块格：一页 COLUMNS × ROWS 格，格 TILE、步长 STEP，左上第一格在 GRID 的左上角。")
    out.add("pub const COLUMNS: usize = %d;" % columns)
    out.add("pub const ROWS: usize = %d;" % rows)
    out.add("pub const TILE: f32 = " + f(tile) + ";")
    out.add("pub const STEP: f32 = " + f(step) + ";")
    out.add("pub const GRID: [f32; 4] = [" + f(left) + ", " + f(top) + ", " + f(grid_w) + ", " + f(grid_h) + "];")
    out.add("")
    if track is not None:
        out.add("/// 右侧滚动条的轨道（点击与拖动都在这个矩形里）。")
        out.add("pub const TRACK: [f32; 4] = " + rect(track, ox, oy) + ";")
        out.add("")
    out.add("/// 底部三个按钮。")
    for const, e in (("BACK", back), ("PREV", prev), ("NEXT", nxt)):
        if e is None:
            continue
        out.add("pub const " + const + ": [f32; 4] = " + rect(e, ox, oy) + ";")
    out.add("")
    emit_at(out)
    emit_labels(out, static, ox, oy)
    emit_draw(out, "；当前外观名由调用方按 NAME_RIGHT 右对齐后画")


def emit_paint(data, out, ox, oy, pw, ph, panel):
    els = data["elements"]
    board = next(e for e in els if e["type"] == "board")
    labels = [e for e in els if e["type"] == "label"]
    buttons = [e for e in els if e["type"] == "button"]
    by_name = {b.get("name"): b for b in buttons if b.get("name")}
    by_text = {b.get("text"): b for b in buttons}

    def named(prefix):
        return [by_name[prefix + str(i)] for i in range(8) if prefix + str(i) in by_name]

    faces = named("face")
    brushes = named("brush")
    eraser = by_name.get("eraser") or by_text.get("橡皮")
    back = by_name.get("back") or by_text.get("返回")
    clear = by_name.get("clear") or by_text.get("清空")
    swatches = [e for e in els if e["type"] == "swatch"]

    emit_common(out, panel, pw, ph)
    out.add("/// 画板：32×32 个像素格，每格 5 px。")
    out.add("pub const CANVAS: [f32; 4] = " + rect(board, ox, oy) + ";")
    out.add("")
    if faces:
        out.add("/// 六面按钮：按 JSON 里的顺序（0 北、1 南、2 西、3 东、4 上、5 下）。")
        out.add("pub const FACES: [[f32; 4]; %d] = [" % len(faces))
        for e in faces:
            out.add("    " + rect(e, ox, oy) + ",")
        out.add("];")
        out.add("")
    if brushes:
        out.add("/// 三档画笔：按 JSON 里的顺序（0 细、1 中、2 粗），位置各用各的。")
        out.add("pub const BRUSHES: [[f32; 4]; %d] = [" % len(brushes))
        for e in brushes:
            out.add("    " + rect(e, ox, oy) + ",")
        out.add("];")
        out.add("")
    if eraser is not None:
        out.add("/// 橡皮。")
        out.add("pub const ERASER: [f32; 4] = " + rect(eraser, ox, oy) + ";")
        out.add("")
    if swatches:
        out.add("/// 16 个色块：按 JSON 里的顺序，和 COLORS 一一对应。")
        out.add("pub const SWATCHES: [[f32; 4]; %d] = [" % len(swatches))
        for e in swatches:
            out.add("    " + rect(e, ox, oy) + ",")
        out.add("];")
        out.add("")
    out.add("/// 底部两个按钮。")
    for const, e in (("BACK", back), ("CLEAR", clear)):
        if e is None:
            continue
        out.add("pub const " + const + ": [f32; 4] = " + rect(e, ox, oy) + ";")
    out.add("")
    emit_at(out)
    emit_labels(out, labels, ox, oy)
    emit_draw(out, "；画板与色块由调用方按 CANVAS / SWATCHES 画")


def main(path, out_path):
    data = json.load(open(path, encoding="utf-8"))
    els = data["elements"]
    panel = next((e for e in els if e["type"] == "panel"), None)
    ox, oy = (panel["x"], panel["y"]) if panel else (0, 0)
    pw, ph = (panel["w"], panel["h"]) if panel else (360, 180)
    page = detect(els)

    out = Out(path)
    if page == "pick":
        emit_pick(data, out, ox, oy, pw, ph, panel)
    elif page == "paint":
        emit_paint(data, out, ox, oy, pw, ph, panel)
    else:
        emit_main(data, out, ox, oy, pw, ph, panel)
    open(out_path, "w", encoding="utf-8").write(out.text())
    print("wrote " + out_path + " (" + page + ")")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
