//! The cube's size and state, and the plain readers for the block entity's compounds: a side is
//! kept in tenths of a block, always even, and a filler cell is cut by naming which tenths of it
//! the cube covers on each axis.

use mod_api::{BlockState, BlockStates, Compound, Tag};

/// A cube's side when the state says nothing usable: one block.
pub(crate) const DEFAULT_TENTHS: i32 = 10;
/// The smallest and largest side, in tenths of a block: 0.2 and 5 blocks.
pub(crate) const MIN_SIZE: i32 = 2;
pub(crate) const MAX_SIZE: i32 = 50;
/// How many cells the largest cube reaches beyond its own cell: five blocks each way at most.
pub(crate) const MAX_OFFSET: i32 = 2;
/// Which tenths of a filler cell the part of the cube that covers it takes, seen from the low side.
const A_KEYS: [&str; 9] = ["a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9"];
/// The same, seen from the high side.
const B_KEYS: [&str; 9] = ["b1", "b2", "b3", "b4", "b5", "b6", "b7", "b8", "b9"];

/// The nearest legal side to a value: even tenths between the limits.
pub(crate) fn clamp_size(value: i32) -> i32 {
    if value.clamp(MIN_SIZE, MAX_SIZE) % 2 == 0 {
        value.clamp(MIN_SIZE, MAX_SIZE)
    } else {
        (value.clamp(MIN_SIZE, MAX_SIZE - 1) / 2 * 2).max(MIN_SIZE)
    }
}

/// The size the buttons step through: one tenth of a block per press.
pub(crate) fn bump_size(value: &mut i32, delta: i32) -> bool {
    let next = clamp_size(*value + 2 * delta);
    if next == *value {
        return false;
    }
    *value = next;
    true
}

pub(crate) fn size_label(size: i32) -> String {
    let side = f64::from(size) / 10.0;
    format!("{side}*{side}*{side}")
}

/// One numeric state property, in tenths.
pub(crate) fn tenths(state: BlockState, meta: &BlockStates<'_>, name: &str) -> i32 {
    state
        .property(meta, name)
        .ok()
        .flatten()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(DEFAULT_TENTHS)
}

/// Whether the state has its collision switched on.
pub(crate) fn solid(state: BlockState, meta: &BlockStates<'_>) -> bool {
    state.property(meta, "solid").ok().flatten() == Some("true")
}

/// Reads an integer field of the block entity data.
pub(crate) fn int(data: &Compound, key: &str, default: i32) -> i32 {
    match data.get(&key.into()) {
        Some(Tag::Int(value)) => *value,
        Some(Tag::Short(value)) => i32::from(*value),
        Some(Tag::Byte(value)) => i32::from(*value),
        _ => default,
    }
}

/// Reads a long field, defaulting to `fallback`.
pub(crate) fn long(data: &Compound, key: &str, fallback: i64) -> i64 {
    match data.get(&key.into()) {
        Some(Tag::Long(value)) => *value,
        Some(Tag::Int(value)) => i64::from(*value),
        _ => fallback,
    }
}

/// Reads a string field of the block entity data.
pub(crate) fn nbt_string(data: &Compound, key: &str) -> Option<String> {
    match data.get(&key.into()) {
        Some(Tag::String(text)) => Some(text.to_string_lossy()),
        _ => None,
    }
}

/// Reads a float the block entity stores as bits.
pub(crate) fn flt(data: &Compound, key: &str) -> f32 {
    match data.get(&key.into()) {
        Some(Tag::Float(bits)) => f32::from_bits(*bits),
        Some(Tag::Double(bits)) => f64::from_bits(*bits) as f32,
        _ => 0.0,
    }
}

fn a_key(tenths: i32) -> Option<&'static str> {
    if tenths <= 0 {
        None
    } else if tenths >= 10 {
        Some("f")
    } else {
        Some(A_KEYS[(tenths - 1) as usize])
    }
}

fn b_key(tenths: i32) -> Option<&'static str> {
    if tenths >= 10 {
        None
    } else if tenths <= 0 {
        Some("f")
    } else {
        Some(B_KEYS[(tenths - 1) as usize])
    }
}

/// How one axis of a filler cell is cut: which tenths of the cell the cube covers there.
pub(crate) fn axis_key(offset: i32, size: i32) -> Option<&'static str> {
    if size <= 0 || offset.abs() > MAX_OFFSET {
        return None;
    }
    if offset == 0 {
        return Some("f");
    }
    let half = size / 2;
    if offset > 0 {
        a_key(5 + half - 10 * offset)
    } else {
        b_key(5 - half - 10 * offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_side_is_always_even_and_inside_the_limits() {
        assert_eq!(clamp_size(0), MIN_SIZE);
        assert_eq!(clamp_size(3), 2);
        assert_eq!(clamp_size(9), 8);
        assert_eq!(clamp_size(10), 10);
        assert_eq!(clamp_size(51), MAX_SIZE);
    }

    #[test]
    fn a_press_moves_one_step_and_stops_at_the_ends() {
        let mut size = DEFAULT_TENTHS;
        assert!(bump_size(&mut size, 1));
        assert_eq!(size, 12);
        assert!(bump_size(&mut size, -1));
        assert_eq!(size, 10);
        let mut small = MIN_SIZE;
        assert!(!bump_size(&mut small, -1));
        assert_eq!(small, MIN_SIZE);
        let mut big = MAX_SIZE;
        assert!(!bump_size(&mut big, 1));
        assert_eq!(big, MAX_SIZE);
    }

    #[test]
    fn the_label_shows_the_side_in_blocks() {
        assert_eq!(size_label(10), "1*1*1");
        assert_eq!(size_label(12), "1.2*1.2*1.2");
    }

    #[test]
    fn a_cell_is_cut_where_the_cube_reaches_it() {
        assert_eq!(axis_key(0, 10), Some("f"));
        assert_eq!(axis_key(1, 10), None);
        assert_eq!(axis_key(1, 20), Some("a5"));
        assert_eq!(axis_key(-1, 20), Some("b5"));
        assert_eq!(axis_key(1, 30), Some("f"));
        assert_eq!(axis_key(3, 20), None);
        assert_eq!(axis_key(0, 0), None);
    }
}
