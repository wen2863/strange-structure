//! What a cube holds: an ordered chain of items, newest first, each turning into a strange one on
//! its own clock. The chain lives in the block entity as a list of compounds and is synced as a
//! whole whenever it changes.

use crate::units::{int, long, nbt_string};
use mod_api::{Compound, Host, Identifier, ItemStack, LevelRef, ServerHost, Tag};

const DIAMOND: &str = "minecraft:diamond";
const SCRAP: &str = "minecraft:netherite_scrap";
const STRANGE_GEM: &str = "strange_structure:strange_gem";
const STRANGE_SHARD: &str = "strange_structure:strange_shard";
const DIAMOND_TICKS: i32 = 100;
const SCRAP_TICKS: i32 = 160;

/// How long an item takes to turn into a strange one, in ticks; zero for the items that
/// only orbit.
pub(crate) fn conversion_ticks(id: &str) -> i32 {
    match id {
        DIAMOND => DIAMOND_TICKS,
        SCRAP => SCRAP_TICKS,
        _ => 0,
    }
}

/// What a convertible item turns into.
pub(crate) fn conversion_result(id: &str) -> Option<&'static str> {
    match id {
        DIAMOND => Some(STRANGE_GEM),
        SCRAP => Some(STRANGE_SHARD),
        _ => None,
    }
}

/// One item the cube holds: what it is, when it went in, and how long it takes to change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChainNode {
    pub(crate) id: String,
    pub(crate) t0: i64,
    pub(crate) dur: i32,
    pub(crate) done: bool,
}

/// Reads the chain out of the block entity data, newest first.
pub(crate) fn read_chain(data: &Compound) -> Vec<ChainNode> {
    let Some(Tag::List(list)) = data.get(&"chain".into()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|tag| {
            let Tag::Compound(fields) = tag else {
                return None;
            };
            Some(ChainNode {
                id: nbt_string(fields, "id")?,
                t0: long(fields, "t0", 0),
                dur: int(fields, "dur", 0),
                done: int(fields, "done", 0) != 0,
            })
        })
        .collect()
}

/// Writes the chain back and asks for a sync on the next tick.
pub(crate) fn write_chain(
    host: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    chain: &[ChainNode],
) -> bool {
    let Some(entity) = host.block_entity(level, pos) else {
        return false;
    };
    let mut patch = Compound::new();
    patch.insert(
        "chain".into(),
        Tag::List(chain.iter().map(node_tag).collect()),
    );
    patch.insert("ps".into(), Tag::Int(1));
    entity.merge_data(host, patch).is_ok()
}

pub(crate) fn node_tag(node: &ChainNode) -> Tag {
    Tag::compound([
        ("id", Tag::String(node.id.as_str().into())),
        ("t0", Tag::Long(node.t0)),
        ("dur", Tag::Int(node.dur)),
        ("done", Tag::Byte(i8::from(node.done))),
    ])
}

/// The stack an item id names, for handing an item back.
pub(crate) fn item_stack(host: &dyn Host, id: &str) -> Option<ItemStack> {
    let ident = Identifier::parse(id).ok()?;
    let key = host.item_key(&ident)?;
    Some(ItemStack::of(key))
}
