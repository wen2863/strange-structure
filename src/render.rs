//! What the client draws for a cube: its own six-face skin, the string of orbs that leads to a
//! converting item, the stored items on their orbit, and the item icons the appearance picker
//! needs. The icons are collected once from the client's registries, since the block entity's item
//! slots are empty on that side.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::{BLOCK, FILL, SKIN};
use mod_api::{BlockEntityDrawContext, ClientHost, CubeFace, DrawContext, ItemKey};

const SKIN_SPRITE: &str = "strange_structure:block/strange_skin";
/// The sprite of the orbs that link a converting item to its cube.
const ORB: &str = "strange_structure:entity/orb";
/// How far outside the cube's surface the stored items circle, in blocks.
pub(crate) const ORBIT_GAP: f32 = 0.85;
/// How far the stored items swing above and below the cube's middle, on top of its half side.
const ORBIT_SWING: f32 = 0.32;
/// Radians the orbit turns each tick.
const ORBIT_RATE: f32 = 0.045;

static ITEM_IDS: Mutex<Option<HashMap<String, ItemKey>>> = Mutex::new(None);
static BLOCK_IDS: Mutex<Option<Vec<String>>> = Mutex::new(None);

/// The appearance name without its namespace, for the narrow value column.
pub(crate) fn short_name(tex: &str) -> &str {
    if tex == SKIN {
        return "本体";
    }
    tex.rsplit(':').next().unwrap_or(tex)
}

/// Whether the game draws this block with a block entity renderer rather than a baked model.
/// A chest (also trapped, ender and copper), a copper golem statue, a banner, a shulker box, a
/// skull or head, a decorated pot, a conduit and a bell have an empty or partial block model, so
/// their look has to come from their item model.
pub(crate) fn item_model_only(tex: &str) -> bool {
    let name = short_name(tex);
    name.ends_with("chest")
        || name.contains("copper_golem_statue")
        || name.ends_with("banner")
        || name.ends_with("shulker_box")
        || name.ends_with("skull")
        || (name.ends_with("_head") && name != "piston_head")
        || matches!(name, "decorated_pot" | "conduit" | "bell")
}

/// How much an item-model appearance has to shrink to fit its cell: the statue's model is taller
/// than one block and a banner's item model is about two blocks tall, everything else is authored
/// at block size.
pub(crate) fn item_model_fit(tex: &str) -> f32 {
    let name = short_name(tex);
    if name.contains("copper_golem_statue") {
        0.7
    } else if name.ends_with("banner") {
        0.55
    } else {
        1.0
    }
}

/// Cuts the text down to a width in GUI pixels, ending it with two dots when it had to lose any:
/// a block id can be far longer than the value column is wide.
pub(crate) fn fit(draw: &dyn DrawContext, text: &str, max_width: f32) -> String {
    if draw.text_width(text) <= max_width {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let mut candidate = out.clone();
        candidate.push(ch);
        candidate.push_str("..");
        if draw.text_width(&candidate) > max_width {
            break;
        }
        out.push(ch);
    }
    out.push_str("..");
    out
}

/// Resolves an item id to its key, on the client: the ids are collected once, because the
/// block entity's own item slots are empty on that side.
pub(crate) fn item_for(host: &dyn ClientHost, id: &str) -> Option<ItemKey> {
    let mut slot = ITEM_IDS.lock().unwrap();
    if slot.is_none() {
        let mut map = HashMap::new();
        for key in host.item_keys() {
            map.insert(key.id(host), key);
        }
        *slot = Some(map);
    }
    slot.as_ref().and_then(|map| map.get(id).copied())
}

/// Every appearance the picker offers: the cube's own skin first, then the game's blocks in id
/// order, the air-like blocks and the cube's own left out because they draw nothing.
pub(crate) fn all_blocks(host: &dyn ClientHost) -> Vec<String> {
    let mut slot = BLOCK_IDS.lock().unwrap();
    if slot.is_none() {
        let mut list = Vec::new();
        for key in host.block_keys() {
            let id = key.id(host);
            if id == BLOCK
                || id == FILL
                || id.ends_with(":air")
                || id.ends_with(":cave_air")
                || id.ends_with(":void_air")
            {
                continue;
            }
            list.push(id);
        }
        list.sort();
        list.insert(0, SKIN.to_owned());
        *slot = Some(list);
    }
    slot.clone().unwrap_or_default()
}

/// A block's name shortened to what one picker tile holds, for the blocks without an icon.
pub(crate) fn tile_label(id: &str) -> String {
    let name = id.rsplit(':').next().unwrap_or(id);
    name.chars().take(4).collect()
}

/// The item that stands for a block the game gives no item of its own, so a wall sign borrows
/// the sign's icon and a wall torch the torch's.
pub(crate) fn stand_in(id: &str) -> Option<String> {
    let (namespace, name) = id.split_once(':')?;
    let (stem, tail) = [
        ("_wall_hanging_sign", "_hanging_sign"),
        ("_wall_sign", "_sign"),
        ("_wall_banner", "_banner"),
        ("_wall_torch", "_torch"),
    ]
    .into_iter()
    .find_map(|(from, to)| name.strip_suffix(from).map(|stem| (stem, to)))?;
    Some(format!("{namespace}:{stem}{tail}"))
}

/// Where a stored item is, in blocks from the block's corner: a circle just outside the cube,
/// each item a step further round than the one below it. The client works it out from the world
/// time it draws with, so an item and the orbs leading to it never drift apart.
pub(crate) fn orbit(tick: i64, side: f32, index: usize) -> [f32; 3] {
    let radius = side * 0.5 + ORBIT_GAP + 0.26 * (index % 3) as f32;
    let swing = (side * 0.25 + ORBIT_SWING).min(0.45);
    let angle = tick as f32 * ORBIT_RATE + index as f32 * 2.1;
    [
        0.5 + radius * angle.cos(),
        0.5 + swing * (angle * 1.3 + index as f32).sin(),
        0.5 + radius * angle.sin(),
    ]
}

/// A point part way along the gently bowed line from a cube to one of its items.
fn bow(from: [f32; 3], to: [f32; 3], t: f32) -> [f32; 3] {
    let span = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (span[0] * span[0] + span[1] * span[1] + span[2] * span[2]).sqrt();
    let lift = 0.20 * length;
    let u = 1.0 - t;
    [
        from[0] * u * u + (from[0] + span[0] * 0.5) * 2.0 * u * t + to[0] * t * t,
        from[1] * u * u + (from[1] + span[1] * 0.5 - lift) * 2.0 * u * t + to[1] * t * t,
        from[2] * u * u + (from[2] + span[2] * 0.5) * 2.0 * u * t + to[2] * t * t,
    ]
}

/// Draws the string of orbs from a cube's side out to a converting item: soft glows, each a pair of
/// crossed quads so it holds from any side, shrinking towards the cube and swelling at the item.
pub(crate) fn draw_orbs(
    draw: &mut dyn BlockEntityDrawContext,
    from: [f32; 3],
    to: [f32; 3],
    half: f32,
    phase: f32,
) {
    let raw = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let far = raw[0].abs().max(raw[1].abs()).max(raw[2].abs());
    if far <= half || far <= f32::EPSILON {
        return;
    }
    let edge = (half / far) * 0.96;
    let start = [
        from[0] + raw[0] * edge,
        from[1] + raw[1] * edge,
        from[2] + raw[2] * edge,
    ];
    let uv = [0.0, 0.0, 16.0, 16.0];
    let count = 7;
    for step in 0..count {
        let t = 0.10 + 0.80 * (step as f32 + 0.5) / count as f32;
        let at = bow(start, to, t);
        let wave = (phase * 0.6 + step as f32 * 1.4).sin() * 0.5 + 0.5;
        let size = (0.042 + 0.048 * t) * (0.72 + 0.55 * wave);
        let at = [at[0], at[1] + (wave - 0.5) * 0.05, at[2]];
        let tint = if step % 2 == 0 { 0xF2DCFF } else { 0xC79BFF };
        draw.quad(
            ORB,
            [
                [at[0] - size, at[1] + size, at[2]],
                [at[0] - size, at[1] - size, at[2]],
                [at[0] + size, at[1] - size, at[2]],
                [at[0] + size, at[1] + size, at[2]],
            ],
            uv,
            tint,
        );
        draw.quad(
            ORB,
            [
                [at[0], at[1] + size, at[2] - size],
                [at[0], at[1] - size, at[2] - size],
                [at[0], at[1] - size, at[2] + size],
                [at[0], at[1] + size, at[2] + size],
            ],
            uv,
            tint,
        );
    }
}

/// Draws the cube's own art: the six faces of the mod sprite in the unit box the pose scales.
pub(crate) fn draw_skin(draw: &mut dyn BlockEntityDrawContext) {
    // One box with the sprite on every face: the helper maps each face to the sprite's whole
    // square whatever the art's resolution, so no per-face UV is needed.
    let face = CubeFace::new(SKIN_SPRITE);
    let _ = draw.cube(
        [0.0, 0.0, 0.0],
        [16.0, 16.0, 16.0],
        [Some(face); 6],
        0xFF_FF_FF,
    );
}
