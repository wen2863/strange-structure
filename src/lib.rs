//! strange_structure: 异构, a strange cube that holds items.
//!
//! The block itself is invisible and unbreakable; its visual is drawn by the block entity. It is
//! always a cube: one state property holds the side in tenths of a block, on all three axes, and a
//! second one switches the collision off while the cube stays visible. The collision comes from two
//! tables: the block own per-state box and invisible filler blocks placed into the neighbouring
//! cells, because the engine only scans one cell beyond a queried box.
//!
//! Ctrl and a right click opens the screen: the state, the size and the appearance, edited over a
//! typed payload so the server keeps the truth.
//!
//! Ctrl and the left click feed the held item into the cube, or take the oldest one back out when
//! the hand is empty. The chain lives in the block entity, newest first, with no limit; each item
//! turns on its own clock and, while it does, the client draws a string of orbs from the cube to it.
//!
//! Ctrl and the space bar take the cube into the hand: it follows the crosshair from a distance the
//! wheel sets and turns by the arrow keys, one step per press, until the keys are pressed again.
//!
//! One cube is owed by every hundredth newly generated chunk and placed on its surface a few ticks
//! later, so world generation never stalls.
//!
//! The code is split by concern:
//!
//! - [`units`]: the side in tenths, how a filler cell is cut, and the plain compound readers.
//! - [`paint`]: the six-face painting, its palette and brushes, and the slabs the client bakes.
//! - [`chain`]: the stored items, their clocks, and the compounds the block entity keeps them in.
//! - [`render`]: the cube's own art, the orbs, the orbit, and the item icons the picker needs.
//! - [`gui_layout`], [`gui_paint`] and [`gui_pick`]: the three pages, generated from the layout
//!   editor's JSON by `tools/layout2rs.py`.
//!
//! Everything else lives here: the hooks, the screen, the payloads, and the server side of moving,
//! absorbing and spawning.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use mod_api::client::widgets::{Button, Outcome, TextField, WidgetId, Widgets};
use mod_api::{
    BlockEntityEvent, BlockEntityPart, BlockEntityRenderBoundingBoxEvent, BlockEntitySubmitEvent,
    BlockEntityTickEvent, BlockEntityTickReply, BlockEntityTicksEvent, BlockEntityUpdateTagEvent,
    BlockPart, BlockState, BlockUpdateFlags, BreakBlockEvent, ChunkLoadEvent, ClientHost,
    ClientTickPreEvent, Compound, DrawContext, Flow, Hand, Heightmap, Hooks, Host, HostBlockStates,
    HostSide, IPayloadContext, InteractionResult, ItemDisplayContext, ItemStack, LevelRef,
    LevelSide, LivingRef, PayloadCodec, PayloadId, Registrar, RegistrationContext,
    RightClickBlockEvent, ScreenCharTypedPreEvent, ScreenKeyPressedPreEvent,
    ScreenMouseButtonPressedPreEvent, ScreenMouseButtonReleasedPreEvent,
    ScreenMouseDraggedPreEvent, ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, ServerHost,
    ServerTickEvent, StateConstraint, Tag, Text, ToClient, ToServer, TriState,
};
use mod_api::{InputKeyEvent, KeyAction, MouseButtonPreEvent, MouseScrollingEvent};

mod chain;
mod gui_layout;
mod gui_paint;
mod gui_pick;
mod paint;
mod render;
mod units;

use chain::*;
use paint::*;
use render::*;
use units::*;

const BLOCK: &str = "strange_structure:strange_block";
const FILL: &str = "strange_structure:strange_fill";
const STONE: &str = "minecraft:stone";
const AIR: &str = "minecraft:air";
/// The cube's own art: a plain block-atlas sprite, drawn on a cube instead of a block model.
const SKIN: &str = "strange_structure:strange_skin";
const SCREEN_ID: &str = "strange_structure:panel";
const STATE_ID: &str = "strange_structure:state";
/// The sprite of the orbs that link a converting item to its cube.
/// One strange cube spawns in every this many newly generated chunks.
const ONE_IN_CHUNKS: u32 = 100;
/// How many queued spawns one server tick may place, so a batch of chunks never stalls the tick.
const SPAWNS_PER_TICK: usize = 4;
const SETTING_DISTANCE: f64 = 8.0;
/// The three pages' panels and controls all come from the layout editor's JSON, compiled into the
/// gui_* modules: change a page in tools/gui-editor.html, regenerate, rebuild.
const PICK_COLUMNS: usize = gui_pick::COLUMNS;
const PICK_ROWS: usize = gui_pick::ROWS;

/// What the screen edits, as one payload: the cube it belongs to and its settings, and the
/// painting when it is one.
#[derive(Clone, Debug, PartialEq, Eq, Default, PayloadCodec)]
struct Config {
    /// 0 writes the values, 1 asks the server for them, 2 writes the painting.
    kind: i32,
    x: i32,
    y: i32,
    z: i32,
    /// 1 while the cube is solid, 0 while it can be walked through.
    solid: i32,
    /// Side in tenths of a block, 2 to 50.
    size: i32,
    /// The block id the cube is drawn as.
    tex: String,
    /// The level it stands in.
    dim: String,
    /// Each of the six faces' 32 by 32 pixels, one `PAINT_DIGITS` character per pixel, in face
    /// order: `p0` is north and `p5` is down, which is the order the drawing, the canvas and the
    /// block entity's own data use. One string per face, because a payload string may hold at
    /// most four thousand bytes and the whole painting is six thousand; empty in the payloads
    /// that do not touch it.
    p0: String,
    p1: String,
    p2: String,
    p3: String,
    p4: String,
    p5: String,
}

/// The open screen: the cube it belongs to, and the values the server confirmed.
struct Open {
    dim: String,
    pos: [i32; 3],
    config: Option<Config>,
}

/// The client's token for Config, from registration.
static SEND: OnceLock<ToServer<Config>> = OnceLock::new();

/// The server's token for the values it answers with.
static REPLY: OnceLock<ToClient<Config>> = OnceLock::new();

/// The open screen's cube and its confirmed values.
static OPEN: Mutex<Option<Open>> = Mutex::new(None);

/// Values the server sent, picked up by the next client tick.
static INCOMING: Mutex<Option<Config>> = Mutex::new(None);

/// The open screen's widgets, made on its first frame.
static SCREEN: Mutex<Option<Screen>> = Mutex::new(None);

/// A change the next client tick sends (the input hooks only read the host).
static PENDING: Mutex<Option<Config>> = Mutex::new(None);

/// Done was pressed: the next client tick closes the screen.
static CLOSE: AtomicBool = AtomicBool::new(false);

/// Item ids to keys, filled the first time the client draws an icon.
/// Newly generated chunks that owe a strange cube, and the chunks already served this session.
static SPAWN_QUEUE: Mutex<Vec<(String, [i32; 2])>> = Mutex::new(Vec::new());
static SPAWNED: Mutex<Vec<(String, [i32; 2])>> = Mutex::new(Vec::new());

/// The mod hooks.
#[derive(Default)]
struct Mod;

/// The strange cube's part.
struct Strange;

/// The part of the invisible blocks that carry the cube's collision.
struct Filler;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let cube = r.block_entity_type(BLOCK).blocks(&[BLOCK]).build();
        r.block_entity_part(&cube, Strange)
            .expect("native block entity type");
        let filler = r.block_entity_type(FILL).blocks(&[FILL]).build();
        r.block_entity_part(&filler, Filler)
            .expect("native block entity type");
        let mut payloads = r.payload_handlers();
        let mut view = payloads.registrar("1");
        let send = view
            .play_to_server_typed::<Config>(PayloadId(SCREEN_ID.to_owned()), |config, context| {
                apply(config, context)
            });
        let _ = SEND.set(send);
        let reply = view.play_to_client_typed::<Config>(
            PayloadId(STATE_ID.to_owned()),
            |config, _context| {
                *INCOMING.lock().unwrap() = Some(config);
                Ok(())
            },
        );
        let _ = REPLY.set(reply);
        let move_send = view
            .play_to_server_typed::<MoveCfg>(PayloadId(MOVE_ID.to_owned()), |cfg, context| {
                move_apply(cfg, context)
            });
        let _ = MOVE_SEND.set(move_send);
        let absorb_send = view
            .play_to_server_typed::<Absorb>(PayloadId(ABSORB_ID.to_owned()), |cfg, context| {
                absorb_apply(cfg, context)
            });
        let _ = ABSORB_SEND.set(absorb_send);
    }

    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        let pos = event.pos;
        let Some(level) = host.level(host.dimension()) else {
            return Flow::Continue;
        };
        let Some(block_key) = host.block_key(BLOCK) else {
            return Flow::Continue;
        };
        let Some(fill) = host.block_key(FILL) else {
            return Flow::Continue;
        };
        let broken = level.block(&*host, pos);
        let cube = if broken == Some(block_key) {
            pos
        } else if broken == Some(fill) {
            // A scaled cube is mostly fillers: hitting one of them counts as hitting the cube.
            let Some(cube) = owner(&*host, level, pos) else {
                return Flow::Continue;
            };
            if level.block(&*host, cube) != Some(block_key) {
                return Flow::Continue;
            }
            level.remove_block(host, cube, false);
            cube
        } else {
            return Flow::Continue;
        };
        // The fillers exist only for the cube's body: once it is gone they are litter that no cube
        // would ever clear, since each of them answers to an owner that is no longer there.
        for dx in -MAX_OFFSET..=MAX_OFFSET {
            for dy in -MAX_OFFSET..=MAX_OFFSET {
                for dz in -MAX_OFFSET..=MAX_OFFSET {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let at = [cube[0] + dx, cube[1] + dy, cube[2] + dz];
                    if level.block(&*host, at) != Some(fill) {
                        continue;
                    }
                    if owner(&*host, level, at) == Some(cube) {
                        level.remove_block(host, at, false);
                    }
                }
            }
        }
        Flow::Continue
    }

    fn right_click_block(&self, host: HostSide, event: &mut RightClickBlockEvent<'_>) -> Flow {
        if event.click.hand != Hand::Main {
            return Flow::Continue;
        }
        let pos = event.click.pos;
        match host {
            HostSide::Client(host) => {
                if !host.modifiers().control() {
                    return Flow::Continue;
                }

                let dim = host.dimension().to_owned();
                let Some(level) = host.level(&dim) else {
                    return Flow::Continue;
                };
                let Some(block) = level.block(&*host, pos) else {
                    return Flow::Continue;
                };
                if !ours(&*host, block) {
                    return Flow::Continue;
                }
                *OPEN.lock().unwrap() = Some(Open {
                    dim: dim.clone(),
                    pos,
                    config: None,
                });
                *SCREEN.lock().unwrap() = None;
                host.mod_screen_open(SCREEN_ID);
                let query = Config {
                    kind: 1,
                    x: pos[0],
                    y: pos[1],
                    z: pos[2],
                    dim,
                    ..Config::default()
                };
                if let Some(send) = SEND.get() {
                    let _ = send.send(host, &query);
                }
            }
            HostSide::Server(_) => {
                // The server cannot see the modifier keys, so it must not swallow the click: a
                // plain right click on a cube has to stay a normal block placement. The screen
                // opens on the client, where Ctrl is known, and the client's cancel already
                // keeps that click from being sent.
                return Flow::Continue;
            }
        }
        *event.use_block = TriState::False;
        *event.use_item = TriState::False;
        *event.cancellation_result = InteractionResult::Success;
        Flow::Cancel
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _event: &mut ClientTickPreEvent) {
        if CLOSE.swap(false, Ordering::Relaxed) {
            host.mod_screen_close(SCREEN_ID);
            *OPEN.lock().unwrap() = None;
            *SCREEN.lock().unwrap() = None;
        }
        if let Some(config) = INCOMING.lock().unwrap().take() {
            if config.kind == 3 {
                // The cube a grabbed filler belongs to: the server resolved it, since the client
                // cannot read a filler's owner. With the real position the follow distance is
                // the one the cube already had, so grabbing it does not make it jump.
                let cube = [config.x, config.y, config.z];
                if let Some(moving) = MOVING.lock().unwrap().as_mut()
                    && moving.dim == config.dim
                {
                    moving.pos = cube;
                    moving.dist = start_distance(&*host, cube);
                }
            } else {
                let mut slot = SCREEN.lock().unwrap();
                let mut open = OPEN.lock().unwrap();
                if let Some(open) = open.as_mut()
                    && open.dim == config.dim
                {
                    open.pos = [config.x, config.y, config.z];
                    open.config = Some(config);
                    *slot = None;
                }
            }
        }

        if MOVE_TICK.fetch_add(1, Ordering::Relaxed) % 2 == 0 {
            let slot = MOVING.lock().unwrap();
            if let Some(moving) = slot.as_ref()
                && let Some(send) = MOVE_SEND.get()
            {
                let cfg = MoveCfg {
                    kind: 1,
                    x: moving.pos[0],
                    y: moving.pos[1],
                    z: moving.pos[2],
                    dim: moving.dim.clone(),
                    dist: moving.dist,
                    yaw: moving.yaw,
                    pitch: moving.pitch,
                    roll: moving.roll,
                };
                let _ = send.send(host, &cfg);
            }
        }
        let Some(config) = PENDING.lock().unwrap().take() else {
            return;
        };
        if let Some(send) = SEND.get() {
            let _ = send.send(host, &config);
        }
    }

    fn input_key(&self, host: &mut dyn ClientHost, event: &mut InputKeyEvent<'_>) {
        if event.key == "key.keyboard.space" {
            if event.action != KeyAction::Press || event.modifiers & CTRL_BIT == 0 {
                return;
            }
            if MOVING.lock().unwrap().take().is_some() {
                if let Some(send) = MOVE_SEND.get() {
                    let _ = send.send(
                        host,
                        &MoveCfg {
                            kind: 2,
                            ..MoveCfg::default()
                        },
                    );
                }
                return;
            }
            let dim = host.dimension().to_owned();
            let Some(level) = host.level(&dim) else {
                return;
            };
            let Some((_, hit)) = host.view_hit() else {
                return;
            };
            // The cube's body is made of fillers on a big cube, and a filler's owner is a block
            // entity field the client never receives: resolve the clicked cell on the server and
            // send its coordinates instead. The server answers with the cube's own position.
            let at = [hit.block_x, hit.block_y, hit.block_z];
            let Some(block) = level.block(&*host, at) else {
                return;
            };
            let id = block.id(&*host);
            if id != BLOCK && id != FILL {
                return;
            }
            let dist = start_distance(&*host, at);
            let moving = Moving {
                dim: dim.clone(),
                pos: at,
                dist,
                yaw: 0,
                pitch: 0,
                roll: 0,
            };
            let cfg = MoveCfg {
                kind: 0,
                x: at[0],
                y: at[1],
                z: at[2],
                dim,
                dist,
                yaw: 0,
                pitch: 0,
                roll: 0,
            };
            *MOVING.lock().unwrap() = Some(moving);
            if let Some(send) = MOVE_SEND.get() {
                let _ = send.send(host, &cfg);
            }
            return;
        }
        let step = match event.key {
            "key.keyboard.left" => (10, 0),
            "key.keyboard.right" => (-10, 0),
            "key.keyboard.up" => (0, -10),
            "key.keyboard.down" => (0, 10),
            _ => return,
        };
        if event.action == KeyAction::Release {
            return;
        }
        let mut slot = MOVING.lock().unwrap();
        let Some(moving) = slot.as_mut() else {
            return;
        };
        moving.yaw = (moving.yaw + step.0).rem_euclid(360);
        moving.pitch = (moving.pitch + step.1).clamp(-90, 90);
    }

    fn mouse_scrolling(&self, host: &mut dyn ClientHost, event: &mut MouseScrollingEvent) -> Flow {
        let steps = event.accumulated[1];
        if steps == 0 {
            return Flow::Continue;
        }
        let mut slot = MOVING.lock().unwrap();
        let Some(moving) = slot.as_mut() else {
            return Flow::Continue;
        };
        moving.dist = (moving.dist + 0.5 * f64::from(steps)).clamp(MIN_DIST, MAX_DIST);
        if let Some(send) = MOVE_SEND.get() {
            let cfg = MoveCfg {
                kind: 1,
                x: moving.pos[0],
                y: moving.pos[1],
                z: moving.pos[2],
                dim: moving.dim.clone(),
                dist: moving.dist,
                yaw: moving.yaw,
                pitch: moving.pitch,
                roll: moving.roll,
            };
            let _ = send.send(host, &cfg);
        }
        Flow::Cancel
    }
    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        if host.mod_screen() != Some(SCREEN_ID) {
            return;
        }
        let [width, height] = event.draw.size();
        let mut slot = SCREEN.lock().unwrap();
        if slot.is_none() {
            let open = OPEN.lock().unwrap();
            let Some(open) = open.as_ref() else {
                return;
            };
            let Some(config) = open.config.clone() else {
                let x = (width / 2.0 - 100.0).floor();
                event.draw.fill([x, 20.0, 200.0, 40.0], 0xC8_10_10_10);
                event
                    .draw
                    .text("奇异方块", [x + 10.0, 26.0], 0xFFFF_FFFF, false);
                event
                    .draw
                    .text("正在读取设置", [x + 10.0, 40.0], 0xFFC0_C0C0, false);
                return;
            };
            *slot = Some(Screen::new(host, width, height, config));
        }
        let Some(screen) = slot.as_mut() else {
            return;
        };
        screen.draw(host, &mut *event.draw);
        let mouse = event.screen.mouse;
        let _ = screen.widgets().draw(&mut *event.draw, host, mouse);
        screen.draw_marks(&mut *event.draw);
    }

    fn screen_mouse_button_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonPressedPreEvent<'_>,
    ) -> Flow {
        let (at, button, modifiers) = (event.screen.mouse, event.button, event.modifiers);
        self.press(host, at, button, modifiers)
    }

    fn screen_mouse_dragged_pre(
        &self,
        _host: &dyn ClientHost,
        event: &mut ScreenMouseDraggedPreEvent<'_>,
    ) -> Flow {
        self.drag(event.screen.mouse)
    }

    fn screen_mouse_button_released_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseButtonReleasedPreEvent<'_>,
    ) -> Flow {
        self.release(host, event.button)
    }

    /// The wheel scrolls the appearance page's grid: a row a notch of the wheel, five with shift.
    fn screen_mouse_scrolled_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenMouseScrolledPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(SCREEN_ID) {
            return Flow::Continue;
        }
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        if screen.view != View::Pick
            || !inside(pick_area(screen.x(), screen.y()), event.screen.mouse)
        {
            return Flow::Continue;
        }
        let amount = (event.scroll[1].abs().round() as usize).clamp(1, PICK_ROWS);
        let step = if event.modifiers & SHIFT_BIT != 0 {
            PICK_ROWS
        } else {
            amount
        };
        if event.scroll[1] > 0.0 {
            screen.scroll = screen.scroll.saturating_sub(step);
        } else if event.scroll[1] < 0.0 {
            let last = pick_scroll_max(screen.visible.len());
            screen.scroll = (screen.scroll + step).min(last);
        } else {
            return Flow::Continue;
        }
        event.cancelled = true;
        Flow::Cancel
    }

    /// The appearance page's search bar takes the keys it is given: typing, deleting and the
    /// arrows, and the grid follows the text as it changes.
    fn screen_key_pressed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenKeyPressedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(SCREEN_ID) {
            return Flow::Continue;
        }
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        if screen.view != View::Pick {
            return Flow::Continue;
        }
        if event.key == "key.keyboard.escape" && screen.picker.focused() == Some(screen.search) {
            screen.picker.set_focus(host, None);
            event.cancelled = true;
            return Flow::Cancel;
        }
        let outcome = screen.picker.key_pressed(host, event.key, event.modifiers);
        if !outcome.handled() {
            return Flow::Continue;
        }
        screen.sync_query();
        event.cancelled = true;
        Flow::Cancel
    }

    /// The characters the search bar's keys carry go the same way as the keys do.
    fn screen_char_typed_pre(
        &self,
        host: &dyn ClientHost,
        event: &mut ScreenCharTypedPreEvent<'_>,
    ) -> Flow {
        if host.mod_screen() != Some(SCREEN_ID) {
            return Flow::Continue;
        }
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        if screen.view != View::Pick {
            return Flow::Continue;
        }
        let outcome = screen.picker.char_typed(host, event.c);
        if !outcome.handled() {
            return Flow::Continue;
        }
        screen.sync_query();
        event.cancelled = true;
        Flow::Cancel
    }

    /// A left click on the cube with a convertible item in hand feeds the item to it.
    fn mouse_button_pre(&self, host: &mut dyn ClientHost, event: &mut MouseButtonPreEvent) -> Flow {
        if event.button != 0 || !event.pressed || event.modifiers & CTRL_BIT == 0 {
            return Flow::Continue;
        }
        let dim = host.dimension().to_owned();
        let Some(level) = host.level(&dim) else {
            return Flow::Continue;
        };
        let Some((_, hit)) = host.view_hit() else {
            return Flow::Continue;
        };
        let at = [hit.block_x, hit.block_y, hit.block_z];
        let Some(block) = level.block(&*host, at) else {
            return Flow::Continue;
        };
        // A big cube is mostly fillers, and the client cannot read a filler's owner. Send the block
        // that was actually hit: the server resolves it to the cube that owns it.
        let id = block.id(&*host);
        if id != BLOCK && id != FILL {
            return Flow::Continue;
        }
        if let Some(send) = ABSORB_SEND.get() {
            let _ = send.send(
                host,
                &Absorb {
                    x: at[0],
                    y: at[1],
                    z: at[2],
                    dim,
                },
            );
        }
        event.cancelled = true;
        Flow::Cancel
    }

    fn chunk_load(&self, _host: &mut dyn ServerHost, event: &mut ChunkLoadEvent<'_>) {
        if !event.new {
            return;
        }
        let handle = spawn_handle(event.dimension, event.chunk);
        if handle % u64::from(ONE_IN_CHUNKS) != 0 {
            return;
        }
        let key = (event.dimension.to_owned(), event.chunk);
        {
            let mut spawned = SPAWNED.lock().unwrap();
            if spawned.contains(&key) {
                return;
            }
            spawned.push(key.clone());
        }
        let mut queue = SPAWN_QUEUE.lock().unwrap();
        if queue.len() < 64 {
            queue.push(key);
        }
    }

    fn server_tick(&self, host: &mut dyn ServerHost, _event: &mut ServerTickEvent<'_>) {
        let mut batch = Vec::new();
        {
            let mut queue = SPAWN_QUEUE.lock().unwrap();
            let take = queue.len().min(SPAWNS_PER_TICK);
            batch.extend(queue.drain(..take));
        }
        for (dimension, chunk) in batch {
            place_spawn(host, &dimension, chunk);
        }
    }
}

impl Mod {
    /// Answers a widget's outcome: the showing page's own buttons act here, everything else
    /// passes through.
    fn outcome(screen: &mut Screen, outcome: Outcome) -> Flow {
        let Outcome::Pressed(id) = outcome else {
            return if outcome.handled() {
                Flow::Cancel
            } else {
                Flow::Continue
            };
        };
        match screen.view {
            View::Main => {
                if id == screen.done {
                    CLOSE.store(true, Ordering::Relaxed);
                    return Flow::Cancel;
                }
                if id == screen.to_pick {
                    screen.view = View::Pick;
                    screen.scroll = screen.reveal();
                    return Flow::Cancel;
                }
                if id == screen.to_paint {
                    screen.view = View::Paint;
                    return Flow::Cancel;
                }
                let changed = if id == screen.solid_on {
                    std::mem::replace(&mut screen.config.solid, 1) != 1
                } else if id == screen.solid_off {
                    std::mem::replace(&mut screen.config.solid, 0) != 0
                } else if id == screen.minus {
                    bump_size(&mut screen.config.size, -1)
                } else if id == screen.plus {
                    bump_size(&mut screen.config.size, 1)
                } else {
                    false
                };
                if changed {
                    return Self::send_settings(screen);
                }
            }
            View::Pick => {
                if id == screen.back_from_pick {
                    screen.view = View::Main;
                } else if id == screen.prev_page {
                    screen.scroll = screen.scroll.saturating_sub(PICK_ROWS);
                } else if id == screen.next_page {
                    let last = pick_scroll_max(screen.visible.len());
                    screen.scroll = (screen.scroll + PICK_ROWS).min(last);
                }
            }
            View::Paint => {
                if id == screen.back_from_paint {
                    screen.view = View::Main;
                    return Flow::Cancel;
                }
                if id == screen.clear {
                    screen.clear_face();
                    return Self::send_paint(screen);
                }
                if let Some(face) = screen.faces.iter().position(|tab| *tab == id) {
                    screen.face = face;
                } else if let Some(brush) = screen.brushes.iter().position(|button| *button == id) {
                    screen.brush = BRUSHES[brush];
                } else if id == screen.eraser {
                    screen.color = 0;
                }
            }
        }
        Flow::Cancel
    }

    /// Picks one of the appearance page's blocks, and goes back to the settings.
    fn pick_tex(screen: &mut Screen, tex: &str) -> Flow {
        let changed = tex != screen.config.tex;
        screen.config.tex = tex.to_owned();
        screen.view = View::Main;
        if changed {
            return Self::send_settings(screen);
        }
        Flow::Cancel
    }

    /// Hands the settings to the server, which keeps the truth.
    fn send_settings(screen: &Screen) -> Flow {
        let mut config = screen.config.clone();
        config.kind = 0;
        [
            config.p0, config.p1, config.p2, config.p3, config.p4, config.p5,
        ] = std::array::from_fn(|_| String::new());
        *PENDING.lock().unwrap() = Some(config);
        Flow::Cancel
    }

    /// Hands the whole painting to the server, which keeps the truth.
    fn send_paint(screen: &Screen) -> Flow {
        let mut config = screen.config.clone();
        config.kind = 2;
        [
            config.p0, config.p1, config.p2, config.p3, config.p4, config.p5,
        ] = encode_paint(&screen.paint);
        *PENDING.lock().unwrap() = Some(config);
        Flow::Cancel
    }

    /// A left press paints on the canvas or picks a colour before the widgets see it, and on the
    /// appearance page it takes the tile under the cursor.
    fn press(&self, host: &dyn ClientHost, at: [f32; 2], button: i32, modifiers: u32) -> Flow {
        if host.mod_screen() != Some(SCREEN_ID) {
            return Flow::Continue;
        }
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        if button == 0 && screen.view == View::Paint {
            if let Some(cell) = cell_at(canvas_rect(screen.x(), screen.y()), at) {
                screen.stroke = Some(cell);
                screen.stroke_to(cell);
                return Flow::Cancel;
            }
            for index in 0..COLORS.len() {
                if inside(swatch_rect(screen.x(), screen.y(), index), at) {
                    screen.color = index as u8 + 1;
                    return Flow::Cancel;
                }
            }
        }
        if button == 0 && screen.view == View::Pick {
            // The scroll bar is grabbed before the tiles: a press on its track or thumb jumps
            // the grid there and keeps following the pointer until the button comes up.
            if inside(pick_track_rect(screen.x(), screen.y()), at) {
                screen.scroll = screen.scroll_for(at[1]);
                screen.scroll_drag = true;
                return Flow::Cancel;
            }
            if let Some(slot) = pick_at(screen.x(), screen.y(), at, screen.scroll)
                && let Some(&index) = screen.visible.get(slot)
            {
                let mut tex = screen.blocks[index].clone();
                if tex == BLOCK {
                    tex = SKIN.to_owned();
                }
                return Self::pick_tex(screen, &tex);
            }
        }
        let outcome = screen.widgets().mouse_pressed(host, at, button, modifiers);
        Self::outcome(screen, outcome)
    }

    /// A drag while the button is down paints from the last cell, so fast strokes leave no gaps.
    fn drag(&self, at: [f32; 2]) -> Flow {
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        if screen.scroll_drag {
            screen.scroll = screen.scroll_for(at[1]);
            return Flow::Cancel;
        }
        if screen.stroke.is_none() {
            return Flow::Continue;
        }
        if let Some(cell) = cell_at(canvas_rect(screen.x(), screen.y()), at) {
            screen.stroke_to(cell);
        }
        Flow::Cancel
    }

    /// Releasing the button either ends a stroke or reaches the widgets.
    fn release(&self, host: &dyn ClientHost, button: i32) -> Flow {
        if host.mod_screen() != Some(SCREEN_ID) {
            return Flow::Continue;
        }
        let mut slot = SCREEN.lock().unwrap();
        let Some(screen) = slot.as_mut() else {
            return Flow::Continue;
        };
        screen.scroll_drag = false;
        if screen.stroke.take().is_some() {
            return Self::send_paint(screen);
        }
        let outcome = screen.widgets().mouse_released(button);
        Self::outcome(screen, outcome)
    }
}

/// Which of the panel's pages is showing: the settings, the appearance picker, or the canvas.
/// The game opens only the mod's own screen, so the pages share it and this field picks one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Main,
    Pick,
    Paint,
}

/// The open panel: the settings, the painting and the widgets that edit both, one page at a
/// time, each page with the widgets it was built with.
struct Screen {
    view: View,
    config: Config,
    /// One byte per pixel of the six faces, in face order: 0 leaves the face bare, 1 to 16 paints
    /// the matching entry of `COLORS`.
    paint: Vec<u8>,
    /// The face the canvas shows, 0 to 5 in `FACE_NAMES` order.
    face: usize,
    /// The brush's width in pixels, one of `BRUSHES`.
    brush: i32,
    /// 0 erases, 1 to 16 paint `COLORS[index - 1]`.
    color: u8,
    /// The last cell the stroke painted, while its button is still down.
    stroke: Option<[i32; 2]>,
    /// Every appearance the picker offers: the cube's own skin first, then the game's blocks.
    blocks: Vec<String>,
    /// Which of `blocks` the search left, in list order: the picker shows only these.
    visible: Vec<usize>,
    /// What the search bar holds, lower-cased for the match.
    query: String,
    /// The first row of the picker's grid that is showing.
    scroll: usize,
    /// The scroll bar's thumb is being dragged, so the pointer scrolls the grid.
    scroll_drag: bool,
    /// The GUI's size in GUI pixels, so a page's panel can be laid out in it.
    gui: [f32; 2],
    settings: Widgets,
    picker: Widgets,
    canvas_widgets: Widgets,
    done: WidgetId,
    solid_on: WidgetId,
    solid_off: WidgetId,
    minus: WidgetId,
    plus: WidgetId,
    to_pick: WidgetId,
    to_paint: WidgetId,
    search: WidgetId,
    back_from_pick: WidgetId,
    prev_page: WidgetId,
    next_page: WidgetId,
    back_from_paint: WidgetId,
    clear: WidgetId,
    faces: [WidgetId; 6],
    brushes: [WidgetId; 3],
    eraser: WidgetId,
}

impl Screen {
    /// The row the grid must start at for the block in use to be on screen.
    fn reveal(&self) -> usize {
        let Some(index) = self
            .visible
            .iter()
            .position(|i| self.blocks[*i] == self.config.tex)
        else {
            return self.scroll;
        };
        (index / PICK_COLUMNS)
            .saturating_sub(2)
            .min(pick_scroll_max(self.visible.len()))
    }

    /// Rebuilds the picker's list from the search bar: every block whose id, or name after the
    /// colon, holds the query; an empty query lists them all. The list scrolls back into range.
    fn refilter(&mut self) {
        let query = self.query.trim().to_lowercase();
        self.visible = self
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, id)| {
                if query.is_empty() {
                    return true;
                }
                if *id == SKIN {
                    return "本体".contains(&query) || "skin".contains(&query);
                }
                let name = id.rsplit(':').next().unwrap_or(id);
                name.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.scroll = self.scroll.min(pick_scroll_max(self.visible.len()));
    }

    /// Takes what the search bar now holds and shows the blocks it leaves.
    fn sync_query(&mut self) {
        let value = self
            .picker
            .text_field(self.search)
            .map(|field| field.value().to_owned())
            .unwrap_or_default();
        if self.query != value {
            self.query = value;
            // A new search starts at the top of its results, not where the old one stood.
            self.scroll = 0;
        }
        self.refilter();
        // The suggestion is a hint for an empty bar: the game would draw it after the text too.
        let hint = self.query.is_empty().then(|| "搜索方块".to_owned());
        if let Some(field) = self.picker.text_field_mut(self.search) {
            field.suggestion = hint;
        }
    }

    /// Where the scroll bar's thumb is, when the list is longer than the grid shows.
    fn scroll_thumb(&self) -> Option<[f32; 4]> {
        let rows = pick_rows(self.visible.len());
        if rows <= PICK_ROWS {
            return None;
        }
        let track = pick_track_rect(self.x(), self.y());
        let height = (track[3] * PICK_ROWS as f32 / rows as f32).max(8.0);
        let last = (rows - PICK_ROWS) as f32;
        let offset = (track[3] - height) * self.scroll as f32 / last;
        Some([track[0], track[1] + offset, track[2], height])
    }

    /// The row the grid goes to when the scroll bar is grabbed at GUI `y`.
    fn scroll_for(&self, y: f32) -> usize {
        let rows = pick_rows(self.visible.len());
        if rows <= PICK_ROWS {
            return 0;
        }
        let track = pick_track_rect(self.x(), self.y());
        let height = (track[3] * PICK_ROWS as f32 / rows as f32).max(8.0);
        let span = track[3] - height;
        if span <= 0.0 {
            return 0;
        }
        let offset = (y - track[1] - height / 2.0).clamp(0.0, span);
        (offset / span * (rows - PICK_ROWS) as f32).round() as usize
    }

    fn new(host: &dyn ClientHost, width: f32, height: f32, config: Config) -> Self {
        let blocks = all_blocks(host);
        let (mx, my) = (
            panel_of(View::Main, width, height)[0],
            panel_of(View::Main, width, height)[1],
        );
        let (kx, ky) = (
            panel_of(View::Pick, width, height)[0],
            panel_of(View::Pick, width, height)[1],
        );
        let (cx, cy) = (
            panel_of(View::Paint, width, height)[0],
            panel_of(View::Paint, width, height)[1],
        );

        let mut settings = Widgets::new();
        let solid_on = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::SOLID_ON),
            Text::literal("刚体"),
        ));
        let solid_off = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::SOLID_OFF),
            Text::literal("虚体"),
        ));
        let minus = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::MINUS),
            Text::literal("-"),
        ));
        let plus = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::PLUS),
            Text::literal("+"),
        ));
        let to_pick = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::TO_PICK),
            Text::literal("选取"),
        ));
        let to_paint = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::TO_PAINT),
            Text::literal("绘制"),
        ));
        let done = settings.add_button(Button::new(
            gui_layout::at(mx, my, gui_layout::DONE),
            Text::literal("完成"),
        ));

        // The appearance page's tiles are hit-tested by hand: a button's own label would sit
        // under the item icon that shows what the tile offers.
        let mut picker = Widgets::new();
        let search =
            picker.add_text_field(TextField::new(gui_pick::at(kx, ky, gui_pick::SEARCH), 24));
        if let Some(field) = picker.text_field_mut(search) {
            field.suggestion = Some("搜索方块".to_owned());
            field.message = "搜索方块".to_owned();
        }
        let back_from_pick = picker.add_button(Button::new(
            gui_pick::at(kx, ky, gui_pick::BACK),
            Text::literal("返回"),
        ));
        let prev_page = picker.add_button(Button::new(
            gui_pick::at(kx, ky, gui_pick::PREV),
            Text::literal("上一页"),
        ));
        let next_page = picker.add_button(Button::new(
            gui_pick::at(kx, ky, gui_pick::NEXT),
            Text::literal("下一页"),
        ));

        // The canvas page: the board, the six faces, three brushes, the eraser, the palette.
        let mut canvas_widgets = Widgets::new();
        let faces = std::array::from_fn(|face| {
            canvas_widgets.add_button(Button::new(
                face_rect(cx, cy, face),
                Text::literal(FACE_NAMES[face]),
            ))
        });
        let brushes = std::array::from_fn(|brush| {
            canvas_widgets.add_button(Button::new(
                brush_rect(cx, cy, brush),
                Text::literal(BRUSH_NAMES[brush]),
            ))
        });
        let eraser =
            canvas_widgets.add_button(Button::new(eraser_rect(cx, cy), Text::literal("橡皮")));
        let clear = canvas_widgets.add_button(Button::new(
            gui_paint::at(cx, cy, gui_paint::CLEAR),
            Text::literal("清空"),
        ));
        let back_from_paint = canvas_widgets.add_button(Button::new(
            gui_paint::at(cx, cy, gui_paint::BACK),
            Text::literal("返回"),
        ));

        let paint = decode_paint(paint_text(&config));
        let visible = (0..blocks.len()).collect();
        Self {
            view: View::Main,
            config,
            paint,
            face: 0,
            brush: BRUSHES[0],
            color: 1,
            stroke: None,
            blocks,
            visible,
            query: String::new(),
            scroll: 0,
            scroll_drag: false,
            gui: [width, height],
            settings,
            picker,
            canvas_widgets,
            done,
            solid_on,
            solid_off,
            minus,
            plus,
            to_pick,
            to_paint,
            search,
            back_from_pick,
            prev_page,
            next_page,
            back_from_paint,
            clear,
            faces,
            brushes,
            eraser,
        }
    }

    /// The showing page's panel, `[x, y, width, height]` in GUI pixels.
    fn panel(&self) -> [f32; 4] {
        panel_of(self.view, self.gui[0], self.gui[1])
    }

    fn x(&self) -> f32 {
        self.panel()[0]
    }

    fn y(&self) -> f32 {
        self.panel()[1]
    }

    /// The showing page's widgets.
    fn widgets(&mut self) -> &mut Widgets {
        match self.view {
            View::Main => &mut self.settings,
            View::Pick => &mut self.picker,
            View::Paint => &mut self.canvas_widgets,
        }
    }

    /// Paints from the stroke's last cell to the given one, so a fast drag leaves no gaps.
    fn stroke_to(&mut self, to: [i32; 2]) {
        if let Some(from) = self.stroke {
            paint_line(&mut self.paint, self.face, from, to, self.brush, self.color);
        }
        self.stroke = Some(to);
    }

    /// Empties the shown face, leaving the other five as they were.
    fn clear_face(&mut self) {
        let start = self.face * CANVAS * CANVAS;
        self.paint[start..start + CANVAS * CANVAS].fill(0);
    }

    /// Draws the showing page: its panel, then what the page itself shows.
    fn draw(&self, host: &dyn ClientHost, draw: &mut dyn DrawContext) {
        let [x, y, _, _] = self.panel();
        match self.view {
            View::Main => self.draw_main(draw),
            View::Pick => {
                gui_pick::draw(draw, x, y);
                let name = short_name(&self.config.tex);
                let name_width = draw.text_width(name);
                draw.text(
                    name,
                    [x + gui_pick::NAME_RIGHT - name_width, y + gui_pick::NAME_Y],
                    gui_pick::NAME_COLOR,
                    false,
                );
                self.draw_tiles(host, draw);
            }
            View::Paint => {
                gui_paint::draw(draw, x, y);
                self.draw_canvas(draw);
            }
        }
    }

    /// Draws the settings page: the state, the size, the appearance's name and the two buttons
    /// that open the appearance's own pages.
    fn draw_main(&self, draw: &mut dyn DrawContext) {
        let (x, y) = (self.x(), self.y());
        gui_layout::draw(draw, x, y);
        let size = size_label(self.config.size);
        let size_width = draw.text_width(&size);
        draw.text(
            &size,
            [
                x + gui_layout::VALUE_RIGHT - size_width,
                y + gui_layout::SIZE_ROW_Y,
            ],
            gui_layout::VALUE_COLOR,
            false,
        );
        let tex = fit(
            &*draw,
            short_name(&self.config.tex),
            gui_layout::VALUE_RIGHT - 40.0,
        );
        let tex_width = draw.text_width(&tex);
        draw.text(
            &tex,
            [
                x + gui_layout::VALUE_RIGHT - tex_width,
                y + gui_layout::TEX_ROW_Y,
            ],
            gui_layout::VALUE_COLOR,
            false,
        );
    }

    /// Draws the appearance page: the blocks' own icons, five rows at a time, and a bar that
    /// says where the shown rows sit in the whole list.
    fn draw_tiles(&self, host: &dyn ClientHost, draw: &mut dyn DrawContext) {
        let (x, y) = (self.x(), self.y());
        let start = self.scroll * PICK_COLUMNS;
        let end = (start + PICK_COLUMNS * PICK_ROWS).min(self.visible.len());
        for slot in start..end {
            let index = self.visible[slot];
            let rect = pick_rect(x, y, slot - start);
            draw.fill(rect, 0xFF20_2020);
            let chosen = self.blocks[index] == self.config.tex
                || (self.blocks[index] == SKIN && self.config.tex == BLOCK);
            outline(draw, rect, if chosen { 0xFFFF_FFFF } else { 0xFF80_8080 });
            let id = self.blocks[index].as_str();
            let id = if id == SKIN { BLOCK } else { id };
            let icon =
                item_for(host, id).or_else(|| stand_in(id).and_then(|name| item_for(host, &name)));
            match icon {
                Some(item) => {
                    draw.item(item, [rect[0] + 4.0, rect[1] + 4.0], rect[2] - 8.0);
                }
                // A block without an item has no icon in the game: its name stands in for one.
                None => {
                    let label = tile_label(id);
                    let label_width = draw.text_width(&label);
                    draw.text(
                        &label,
                        [rect[0] + (rect[2] - label_width) / 2.0, rect[1] + 10.0],
                        0xFFC0_C0C0,
                        false,
                    );
                }
            }
        }
        // An empty result set should say so, not look like a grid that failed to draw.
        if self.visible.is_empty() {
            let area = pick_area(x, y);
            let label = "没有匹配的方块";
            let width = draw.text_width(label);
            draw.text(
                label,
                [
                    area[0] + (area[2] - width) / 2.0,
                    area[1] + area[3] / 2.0 - 5.0,
                ],
                0xFF80_8080,
                false,
            );
        }
        let track = pick_track_rect(x, y);
        draw.fill(track, 0xFF30_3030);
        if let Some(thumb) = self.scroll_thumb() {
            draw.fill(thumb, 0xFFA0_A0A0);
        }
    }

    /// Draws the canvas of the shown face beside the controls.
    fn draw_canvas(&self, draw: &mut dyn DrawContext) {
        let (x, y) = (self.x(), self.y());
        let canvas = canvas_rect(x, y);
        draw.fill(canvas, 0xFF2A2A2A);
        // A faint cell grid, so a pixel's place is plain on an empty canvas.
        for index in 1..CANVAS {
            let offset = index as f32 * CANVAS_CELL;
            draw.fill([canvas[0] + offset, canvas[1], 1.0, canvas[3]], 0x40FF_FFFF);
            draw.fill([canvas[0], canvas[1] + offset, canvas[2], 1.0], 0x40FF_FFFF);
        }
        let pixels = &self.paint[self.face * CANVAS * CANVAS..(self.face + 1) * CANVAS * CANVAS];
        for (px, py, width, height, color) in runs(pixels) {
            draw.fill(
                [
                    canvas[0] + px as f32 * CANVAS_CELL,
                    canvas[1] + py as f32 * CANVAS_CELL,
                    width as f32 * CANVAS_CELL,
                    height as f32 * CANVAS_CELL,
                ],
                0xFF00_0000 | COLORS[color as usize - 1],
            );
        }
        outline(draw, canvas, 0xFF80_8080);
        for index in 0..COLORS.len() {
            draw.fill(swatch_rect(x, y, index), 0xFF00_0000 | COLORS[index]);
        }
    }

    /// Marks what the showing page's next press would act on.
    fn draw_marks(&self, draw: &mut dyn DrawContext) {
        match self.view {
            View::Main => {
                let id = if self.config.solid == 0 {
                    self.solid_off
                } else {
                    self.solid_on
                };
                if let Some(button) = self.settings.button(id) {
                    outline(draw, button.rect, 0xFFFF_FFFF);
                }
            }
            View::Pick => {}
            View::Paint => {
                outline(draw, face_rect(self.x(), self.y(), self.face), 0xFFFF_FFFF);
                if let Some(brush) = BRUSHES.iter().position(|width| *width == self.brush) {
                    outline(draw, brush_rect(self.x(), self.y(), brush), 0xFFFF_FFFF);
                }
                if self.color == 0 {
                    outline(draw, eraser_rect(self.x(), self.y()), 0xFFFF_FFFF);
                } else {
                    outline(
                        draw,
                        swatch_rect(self.x(), self.y(), self.color as usize - 1),
                        0xFFFF_FFFF,
                    );
                }
            }
        }
    }
}

/// Whether the block belongs to this mod.
fn ours(host: &dyn Host, block: mod_api::BlockKey) -> bool {
    let id = block.id(host);
    id == BLOCK || id == FILL
}

/// The cube a filler block belongs to, when it carries one.
fn owner(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<[i32; 3]> {
    let entity = host.block_entity(level, pos)?;
    let data = entity.data(host)?;
    let x = int(&data, "ox", 0);
    let y = int(&data, "oy", 0);
    let z = int(&data, "oz", 0);
    Some([x, y, z])
}

/// The cube's settings, as the server knows them.
fn load(host: &dyn Host, level: LevelRef, pos: [i32; 3], dim: &str) -> Config {
    let mut config = Config {
        kind: 0,
        x: pos[0],
        y: pos[1],
        z: pos[2],
        solid: 1,
        size: DEFAULT_TENTHS,
        tex: SKIN.to_owned(),
        p0: String::new(),
        p1: String::new(),
        p2: String::new(),
        p3: String::new(),
        p4: String::new(),
        p5: String::new(),
        dim: dim.to_owned(),
    };
    if let Some(state) = level.block_state(host, pos) {
        let meta = host.block_states();
        config.size = clamp_size(tenths(state, &meta, "size"));
        config.solid = i32::from(solid(state, &meta));
    }
    if let Some(entity) = host.block_entity(level, pos)
        && let Some(data) = entity.data(host)
    {
        if let Some(Tag::String(tex)) = data.get(&"tex".into()) {
            config.tex = tex.to_string_lossy();
        }
        let mut faces = [
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        for (face, slot) in faces.iter_mut().enumerate() {
            if let Some(Tag::String(text)) = data.get(&format!("p{face}").as_str().into()) {
                *slot = text.to_string_lossy();
            }
        }
        [
            config.p0, config.p1, config.p2, config.p3, config.p4, config.p5,
        ] = faces;
    }
    config
}

/// The payload handler: answers a query, or stores the settings on the cube.
fn apply(config: Config, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let Ok(player) = context.player() else {
        return Ok(());
    };
    let Ok(host) = context.server_host() else {
        return Ok(());
    };
    let sender = player
        .dimension(&*host)
        .map_or_else(String::new, |dim| dim.to_string());
    if sender != config.dim {
        let _ = player.tell(host, Text::literal("异构：不在同一维度"));
        return Ok(());
    }
    let Some(level) = host.level(&config.dim) else {
        return Ok(());
    };
    let Some(block_key) = host.block_key(BLOCK) else {
        return Ok(());
    };
    let fill = host.block_key(FILL);
    let clicked = [config.x, config.y, config.z];
    let pos = if level.block(&*host, clicked).is_some() && level.block(&*host, clicked) == fill {
        owner(&*host, level, clicked).unwrap_or(clicked)
    } else {
        clicked
    };
    if level.block(&*host, pos) != Some(block_key) {
        return Ok(());
    }
    let near = player.entity().position(&*host).is_some_and(|at| {
        let (dx, dy, dz) = (
            at[0] - f64::from(pos[0]),
            at[1] - f64::from(pos[1]),
            at[2] - f64::from(pos[2]),
        );
        dx * dx + dy * dy + dz * dz <= SETTING_DISTANCE * SETTING_DISTANCE
    });
    if !near {
        let _ = player.tell(host, Text::literal("异构：距离太远"));
        return Ok(());
    }
    if config.kind == 1 {
        let truth = load(&*host, level, pos, &config.dim);
        if let Some(reply) = REPLY.get() {
            let _ = reply.reply(context, &truth);
        }
        return Ok(());
    }
    if config.kind == 2 {
        let halves = paint_text(&config);
        if paint_valid(halves)
            && let Some(entity) = host.block_entity(level, pos)
        {
            let mut patch = Compound::new();
            for (face, text) in halves.iter().enumerate() {
                patch.insert(
                    format!("p{face}").as_str().into(),
                    Tag::String((*text).into()),
                );
            }
            let _ = entity.merge_data(host, patch);
            host.block_entity_sync(pos);
        }
        return Ok(());
    }
    let size = clamp_size(config.size);
    let is_solid = config.solid != 0;
    let tex = if config.tex.is_empty() {
        SKIN.to_owned()
    } else {
        config.tex.clone()
    };
    if tex != SKIN && host.block_key(&tex).is_none() {
        let _ = player.tell(host, Text::literal("异构：没有这个方块"));
        return Ok(());
    }
    let state = {
        let meta = host.block_states();
        BlockState::of(
            &meta,
            block_key,
            &[
                StateConstraint::new("size", size.to_string()),
                StateConstraint::new("solid", if is_solid { "true" } else { "false" }),
            ],
        )
        .ok()
    };
    if let Some(state) = state
        && level.block_state(&*host, pos) != Some(state)
    {
        let _ = level.set_block(
            host,
            pos,
            state,
            BlockUpdateFlags::CLIENTS | BlockUpdateFlags::NEIGHBORS,
        );
    }
    if let Some(entity) = host.block_entity(level, pos) {
        let mut patch = Compound::new();
        patch.insert("size".into(), Tag::Int(size));
        patch.insert("solid".into(), Tag::Byte(i8::from(is_solid)));
        patch.insert("tex".into(), Tag::String(tex.as_str().into()));
        let _ = entity.merge_data(host, patch);
    }
    host.block_entity_sync(pos);
    Ok(())
}

/// The absorb handler: takes the item the asking player holds into the cube it names.
fn absorb_apply(cfg: Absorb, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let Ok(player) = context.player() else {
        return Ok(());
    };
    let Ok(host) = context.server_host() else {
        return Ok(());
    };
    let sender = player
        .dimension(&*host)
        .map_or_else(String::new, |dim| dim.to_string());
    if sender != cfg.dim {
        return Ok(());
    }
    let Some(level) = host.level(&cfg.dim) else {
        return Ok(());
    };
    // The click may land on one of the cube's fillers: resolve it to the cube itself.
    let clicked = [cfg.x, cfg.y, cfg.z];
    let Some(pos) = cube_at(&*host, level, clicked) else {
        return Ok(());
    };
    let near = player.entity().position(&*host).is_some_and(|at| {
        let (dx, dy, dz) = (
            at[0] - f64::from(clicked[0]),
            at[1] - f64::from(clicked[1]),
            at[2] - f64::from(clicked[2]),
        );
        dx * dx + dy * dy + dz * dz <= MOVE_REACH * MOVE_REACH
    });
    if !near {
        let _ = player.tell(host, Text::literal("异构：距离太远"));
        return Ok(());
    }
    let Some(entity) = host.block_entity(level, pos) else {
        return Ok(());
    };
    let Some(data) = entity.data(&*host) else {
        return Ok(());
    };
    let mut chain = read_chain(&data);
    let held = LivingRef::new(player.entity()).hand_item(&*host, Hand::Main);
    let tick = level.game_time(&*host).unwrap_or(0);
    let Some(held) = held else {
        // An empty hand takes the oldest item back, out of the chain's tail.
        let Some(node) = chain.pop() else {
            let _ = player.tell(host, Text::literal("异构：里面没有物品"));
            return Ok(());
        };
        if !write_chain(host, level, pos, &chain) {
            return Ok(());
        }
        let at = player.entity().position(&*host).unwrap_or([
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ]);
        if let Some(stack) = item_stack(&*host, &node.id) {
            let _ = level.spawn_item(host, at[0], at[1] + 0.6, at[2], stack, 0);
        }
        host.block_entity_sync(pos);
        return Ok(());
    };
    // A held item goes in at the head, so the newest is always first.
    let id = held.item().id(&*host);
    chain.insert(
        0,
        ChainNode {
            id: id.clone(),
            t0: tick,
            dur: conversion_ticks(&id),
            done: false,
        },
    );
    if !write_chain(host, level, pos, &chain) {
        return Ok(());
    }
    let left = if held.count() > 1 {
        ItemStack::new(held.item(), held.count() - 1)
    } else {
        None
    };
    let _ = LivingRef::new(player.entity()).set_hand_item(host, Hand::Main, left);
    host.block_entity_sync(pos);
    Ok(())
}

/// One page's panel, `[x, y, width, height]` in GUI pixels, centred in the GUI.
fn panel_of(view: View, width: f32, height: f32) -> [f32; 4] {
    let (panel_width, panel_height) = match view {
        View::Main => (gui_layout::PANEL_W, gui_layout::PANEL_H),
        View::Pick => (gui_pick::PANEL_W, gui_pick::PANEL_H),
        View::Paint => (gui_paint::PANEL_W, gui_paint::PANEL_H),
    };
    [
        ((width - panel_width) / 2.0).floor(),
        ((height - panel_height) / 2.0).floor(),
        panel_width,
        panel_height,
    ]
}

/// How many rows the picker's grid holds for that many blocks.
fn pick_rows(len: usize) -> usize {
    len.div_ceil(PICK_COLUMNS)
}

/// The last first row the picker may show, so its last row ends at the grid's bottom.
fn pick_scroll_max(len: usize) -> usize {
    pick_rows(len).saturating_sub(PICK_ROWS)
}

/// The picker's grid, where the wheel scrolls.
fn pick_area(x: f32, y: f32) -> [f32; 4] {
    gui_pick::at(x, y, gui_pick::GRID)
}

/// The picker's scroll bar, right of the grid.
fn pick_track_rect(x: f32, y: f32) -> [f32; 4] {
    gui_pick::at(x, y, gui_pick::TRACK)
}

/// One tile of the picker's grid, counted from the first shown row.
fn pick_rect(x: f32, y: f32, slot: usize) -> [f32; 4] {
    [
        x + gui_pick::GRID[0] + (slot % PICK_COLUMNS) as f32 * gui_pick::STEP,
        y + gui_pick::GRID[1] + (slot / PICK_COLUMNS) as f32 * gui_pick::STEP,
        gui_pick::TILE,
        gui_pick::TILE,
    ]
}

/// The picker's grid slot under the cursor, if any: a position in the shown list.
fn pick_at(x: f32, y: f32, at: [f32; 2], scroll: usize) -> Option<usize> {
    if !inside(pick_area(x, y), at) {
        return None;
    }
    let column = ((at[0] - x - gui_pick::GRID[0]) / gui_pick::STEP) as usize;
    let row = ((at[1] - y - gui_pick::GRID[1]) / gui_pick::STEP) as usize;
    if column >= PICK_COLUMNS || row >= PICK_ROWS {
        return None;
    }
    Some((scroll + row) * PICK_COLUMNS + column)
}

/// One face button's rectangle, from the JSON: the six run down a column.
fn face_rect(x: f32, y: f32, face: usize) -> [f32; 4] {
    gui_paint::at(x, y, gui_paint::FACES[face])
}

/// One brush button's rectangle, from the JSON: the JSON's order is the brush order.
fn brush_rect(x: f32, y: f32, brush: usize) -> [f32; 4] {
    gui_paint::at(x, y, gui_paint::BRUSHES[brush])
}

/// The eraser button's rectangle, right of the coarsest brush.
fn eraser_rect(x: f32, y: f32) -> [f32; 4] {
    gui_paint::at(x, y, gui_paint::ERASER)
}

/// One palette swatch, in the JSON's order (it is the `COLORS` order).
fn swatch_rect(x: f32, y: f32, index: usize) -> [f32; 4] {
    gui_paint::at(x, y, gui_paint::SWATCHES[index])
}

/// Draws a one-pixel frame, so the chosen face, brush and colour read at a glance.
fn outline(draw: &mut dyn DrawContext, rect: [f32; 4], argb: u32) {
    let [x, y, width, height] = rect;
    draw.fill([x, y, width, 1.0], argb);
    draw.fill([x, y + height - 1.0, width, 1.0], argb);
    draw.fill([x, y, 1.0, height], argb);
    draw.fill([x + width - 1.0, y, 1.0, height], argb);
}

fn inside(rect: [f32; 4], at: [f32; 2]) -> bool {
    at[0] >= rect[0] && at[1] >= rect[1] && at[0] < rect[0] + rect[2] && at[1] < rect[1] + rect[3]
}

#[mod_api::horizon_part]
impl BlockPart for Strange {}

#[mod_api::horizon_part]
impl BlockPart for Filler {}

#[mod_api::horizon_part]
impl BlockEntityPart for Filler {
    fn block_entity(
        &self,
        _level: LevelSide,
        _event: &mut BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        Some(Compound::new())
    }
}

#[mod_api::horizon_part]
impl BlockEntityPart for Strange {
    fn block_entity(
        &self,
        _level: LevelSide,
        _event: &mut BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        let mut data = Compound::new();
        data.insert("size".into(), Tag::Int(DEFAULT_TENTHS));
        data.insert("solid".into(), Tag::Byte(1));
        data.insert("tex".into(), Tag::String(SKIN.into()));
        Some(data)
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        *event.reply = BlockEntityTickReply::Unchanged;
        let pos = event.pos;
        let Some(level) = host.level(host.dimension()) else {
            return;
        };
        let Some(state) = level.block_state(&*host, pos) else {
            return;
        };
        let (size, is_solid) = {
            let meta = host.block_states();
            (
                clamp_size(tenths(state, &meta, "size")),
                solid(state, &meta),
            )
        };
        // A conversion written last tick is published to the clients now, when the block
        // entity already holds it: a sync inside the same tick would send the old tag.
        let tick = level.game_time(&*host).unwrap_or(0);
        if int(&event.entity.nbt, "ps", 0) == 1 {
            event.entity.nbt.insert("ps".into(), Tag::Int(0));
            host.block_entity_sync(pos);
            *event.reply = BlockEntityTickReply::Changed;
        }
        // Every item in the chain turns on its own clock; the turning only shows once it is done.
        let mut chain = read_chain(&event.entity.nbt);
        let mut changed = false;
        for node in chain.iter_mut() {
            if node.done || node.dur <= 0 {
                continue;
            }
            if tick - node.t0 >= i64::from(node.dur)
                && let Some(to) = conversion_result(&node.id)
            {
                node.id = to.to_owned();
                node.done = true;
                changed = true;
            }
        }
        if changed {
            event.entity.nbt.insert(
                "chain".into(),
                Tag::List(chain.iter().map(node_tag).collect()),
            );
            event.entity.nbt.insert("ps".into(), Tag::Int(1));
            *event.reply = BlockEntityTickReply::Changed;
        }
        // The stamp remembers both settings: the size, then whether the cube is solid.
        let stamp_value = size * 2 + i32::from(is_solid);
        if int(&event.entity.nbt, "bs", 0) == stamp_value {
            return;
        }
        let Some(fill) = host.block_key(FILL) else {
            return;
        };
        let air = host.block_key(AIR);
        let mut plan: Vec<([i32; 3], Option<BlockState>)> = Vec::new();
        {
            let meta = host.block_states();
            for dx in -MAX_OFFSET..=MAX_OFFSET {
                for dy in -MAX_OFFSET..=MAX_OFFSET {
                    for dz in -MAX_OFFSET..=MAX_OFFSET {
                        if dx == 0 && dy == 0 && dz == 0 {
                            continue;
                        }
                        let at = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
                        let cuts = (axis_key(dx, size), axis_key(dy, size), axis_key(dz, size));
                        let target = match cuts {
                            (Some(kx), Some(ky), Some(kz)) => BlockState::of(
                                &meta,
                                fill,
                                &[
                                    StateConstraint::new("px", kx),
                                    StateConstraint::new("py", ky),
                                    StateConstraint::new("pz", kz),
                                    StateConstraint::new(
                                        "solid",
                                        if is_solid { "true" } else { "false" },
                                    ),
                                ],
                            )
                            .ok(),
                            _ => None,
                        };
                        plan.push((at, target));
                    }
                }
            }
        }
        for (at, target) in plan {
            let current = level.block(&*host, at);
            let is_filler = current.is_some_and(|block| block == fill);
            if is_filler {
                // Another cube's filler is never touched.
                if owner(&*host, level, at).is_some_and(|cube| cube != pos) {
                    continue;
                }
                match target {
                    Some(target) => {
                        if level.block_state(&*host, at) == Some(target) {
                            continue;
                        }
                        let _ = level.set_block(host, at, target, BlockUpdateFlags::CLIENTS);
                        stamp(host, level, at, pos);
                    }
                    None => {
                        level.remove_block(host, at, false);
                    }
                }
                continue;
            }
            let vacant = air.is_some_and(|air| current.is_some_and(|block| block == air));
            let Some(target) = target else {
                continue;
            };
            if !vacant {
                continue;
            }
            let _ = level.set_block(host, at, target, BlockUpdateFlags::CLIENTS);
            stamp(host, level, at, pos);
        }
        event.entity.nbt.insert("bs".into(), Tag::Int(stamp_value));
        *event.reply = BlockEntityTickReply::Changed;
    }

    fn block_entity_update_tag(
        &self,
        event: &mut BlockEntityUpdateTagEvent<'_>,
    ) -> Option<Compound> {
        let mut data = Compound::new();
        for key in [
            "tex", "ox", "oy", "oz", "ry", "rp", "rr", "chain", "p0", "p1", "p2", "p3", "p4", "p5",
        ] {
            if let Some(value) = event.entity.nbt.get(&key.into()) {
                data.insert(key.into(), value.clone());
            }
        }
        Some(data)
    }

    fn block_entity_render_bounding_box(
        &self,
        host: &dyn ClientHost,
        event: &mut BlockEntityRenderBoundingBoxEvent<'_>,
    ) {
        let size = {
            let meta = host.block_states();
            tenths(event.state, &meta, "size")
        };
        if size <= 0 {
            return;
        }
        let half = size as f32 / 20.0;
        let pad = half + ORBIT_GAP + 0.6;
        let low = (0.5 - pad).min(-1.0);
        let high = (0.5 + pad).max(2.0);
        *event.bounds = [low, low, low, high, high, high];
    }

    fn block_entity_submit(&self, host: &dyn ClientHost, event: &mut BlockEntitySubmitEvent<'_>) {
        let size = {
            let meta = host.block_states();
            tenths(event.state, &meta, "size")
        };
        if size <= 0 {
            return;
        }
        let scale = size as f32 / 10.0;
        let tex = match event.entity.nbt.get(&"tex".into()) {
            Some(Tag::String(tex)) => tex.to_string_lossy(),
            _ => SKIN.to_owned(),
        };
        let model = if tex == SKIN {
            None
        } else {
            host.block_key(&tex).or_else(|| host.block_key(STONE))
        };
        let nbt = &event.entity.nbt;
        let off = [flt(nbt, "ox"), flt(nbt, "oy"), flt(nbt, "oz")];
        let yaw = int(nbt, "ry", 0) as f32;
        let pitch = int(nbt, "rp", 0) as f32;
        let roll = int(nbt, "rr", 0) as f32;
        let draw = &mut *event.draw;
        draw.push();
        draw.translate([
            0.5 - scale * 0.5 + off[0],
            0.5 - scale * 0.5 + off[1],
            0.5 - scale * 0.5 + off[2],
        ]);
        draw.scale([scale, scale, scale]);
        let pivot = [0.5, 0.5, 0.5];
        if yaw != 0.0 {
            draw.rotate_about([0.0, 1.0, 0.0], yaw, pivot);
        }
        if pitch != 0.0 {
            draw.rotate_about([1.0, 0.0, 0.0], pitch, pivot);
        }
        if roll != 0.0 {
            draw.rotate_about([0.0, 0.0, 1.0], roll, pivot);
        }
        match model {
            Some(model) => {
                // A chest or the copper golem statue is drawn by a block entity renderer and has no
                // baked geometry: its block model is empty, so only its item model can show it.
                if item_model_only(&tex) {
                    match item_for(host, &tex) {
                        Some(key) => {
                            let stack = ItemStack::of(key);
                            // An item is drawn around the pose's origin, a block from its corner: move
                            // the origin to the middle of the cell so the model fills it.
                            draw.push();
                            draw.translate([0.5, 0.5, 0.5]);
                            let fit = item_model_fit(&tex);
                            if fit != 1.0 {
                                draw.scale([fit, fit, fit]);
                            }
                            let _ = draw.item(&stack, ItemDisplayContext::None, None, 0);
                            draw.pop();
                        }
                        None => {
                            let _ = draw.block(model, &[]);
                        }
                    }
                } else {
                    let _ = draw.block(model, &[]);
                }
            }
            None => draw_skin(&mut *draw),
        }
        if let Some(paint) = read_paint(nbt) {
            draw_paint(&mut *draw, &paint);
        }
        draw.pop();
        // The stored items run their circles outside the cube, on the client's clock, so the
        // orbs drawn to a converting one line up with it exactly.
        let chain = read_chain(nbt);
        if !chain.is_empty() {
            let time = draw.game_time();
            for (index, node) in chain.iter().enumerate() {
                let at = orbit(time, scale, index);
                let point = [at[0] + off[0], at[1] + off[1], at[2] + off[2]];
                // A converting item keeps a string of orbs back to the cube; a finished one stops.
                if !node.done && node.dur > 0 {
                    let center = [0.5 + off[0], 0.5 + off[1], 0.5 + off[2]];
                    draw_orbs(&mut *draw, center, point, scale * 0.5, time as f32);
                }
                let Some(key) = item_for(host, &node.id) else {
                    continue;
                };
                let stack = ItemStack::of(key);
                draw.push();
                draw.translate(point);
                draw.scale([0.6, 0.6, 0.6]);
                let _ = draw.item(&stack, ItemDisplayContext::Ground, None, index as i32);
                draw.pop();
            }
        } else {
            // Nothing circles the cube, so its geometry can be cached until the cube changes.
            draw.keep();
        }
    }
}

/// A stable handle of a chunk: the same chunk always answers the same, so a cube is placed once.
fn spawn_handle(dimension: &str, chunk: [i32; 2]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in dimension.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    for value in chunk {
        hash ^= u64::from(value as u32);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash ^= hash >> 33;
    hash
}

/// Places the strange cube owed by a chunk above its surface, unless something already stands there.
fn place_spawn(host: &mut dyn ServerHost, dimension: &str, chunk: [i32; 2]) {
    let handle = spawn_handle(dimension, chunk);
    let x = chunk[0] * 16 + (handle % 16) as i32;
    let z = chunk[1] * 16 + ((handle >> 8) % 16) as i32;
    let Ok(heights) = host.surface_heights(dimension, &[(x, z)], Heightmap::WorldSurface) else {
        return;
    };
    let Some(Some(surface)) = heights.first() else {
        return;
    };
    let at = [x, *surface + 1, z];
    let Some(level) = host.level(dimension) else {
        return;
    };
    let Some(state) = level.block_state(&*host, at) else {
        return;
    };
    let occupied = state.block().id(&*host);
    if occupied != AIR && occupied != "minecraft:cave_air" {
        return;
    }
    let Some(key) = host.block_key(BLOCK) else {
        return;
    };
    let meta = host.block_states();
    let Ok(target) = BlockState::of(&meta, key, &[]) else {
        return;
    };
    drop(meta);
    let _ = level.set_block(
        host,
        at,
        target,
        BlockUpdateFlags::CLIENTS | BlockUpdateFlags::NEIGHBORS,
    );
}

/// Marks a filler block as belonging to the cube at the given position.
fn stamp(host: &mut dyn ServerHost, level: LevelRef, at: [i32; 3], cube: [i32; 3]) {
    let Some(entity) = host.block_entity(level, at) else {
        return;
    };
    let mut patch = Compound::new();
    patch.insert("ox".into(), Tag::Int(cube[0]));
    patch.insert("oy".into(), Tag::Int(cube[1]));
    patch.insert("oz".into(), Tag::Int(cube[2]));
    let _ = entity.merge_data(host, patch);
}

// ---- M3: the cube follows the crosshair ----

/// What one move payload carries: the cube, where the client holds it, and its aim.
#[derive(Clone, Debug, PartialEq, Default, PayloadCodec)]
struct MoveCfg {
    /// 0 enters the mode, 1 is a follow step, 2 leaves it.
    kind: i32,
    x: i32,
    y: i32,
    z: i32,
    dim: String,
    dist: f64,
    yaw: i32,
    pitch: i32,
    roll: i32,
}

/// The cube the client is moving, as it last knew it.
struct Moving {
    dim: String,
    pos: [i32; 3],
    dist: f64,
    yaw: i32,
    pitch: i32,
    roll: i32,
}

const MOVE_ID: &str = "strange_structure:move";
const ABSORB_ID: &str = "strange_structure:absorb";
const CTRL_BIT: u32 = 2;

/// The shift bit of a hook's modifiers.
const SHIFT_BIT: u32 = 1;
/// How close the cube must be to start moving it, in blocks.
const MOVE_REACH: f64 = 8.0;
const MIN_DIST: f64 = 2.0;
const MAX_DIST: f64 = 12.0;

/// What one absorb payload carries: the cube that should take the asking player's item.
#[derive(Clone, Debug, PartialEq, Default, PayloadCodec)]
struct Absorb {
    x: i32,
    y: i32,
    z: i32,
    dim: String,
}

/// The client's token for MoveCfg.
static MOVE_SEND: OnceLock<ToServer<MoveCfg>> = OnceLock::new();

/// The client's token for Absorb.
static ABSORB_SEND: OnceLock<ToServer<Absorb>> = OnceLock::new();

/// The cube this client is moving.
static MOVING: Mutex<Option<Moving>> = Mutex::new(None);

/// Counts client ticks, so follow steps go out every other tick.
static MOVE_TICK: AtomicU64 = AtomicU64::new(0);

/// Who is moving which cube, on the server.
static DRIVERS: Mutex<Vec<(u128, String, [i32; 3])>> = Mutex::new(Vec::new());

/// The cube at `pos`, or the cube that owns the filler at `pos`.
fn cube_at(host: &dyn Host, level: LevelRef, pos: [i32; 3]) -> Option<[i32; 3]> {
    let block = level.block(host, pos)?;
    if block.id(host) == BLOCK {
        return Some(pos);
    }
    if block.id(host) == FILL {
        return owner(host, level, pos);
    }
    None
}

/// How far from the player's eye the grabbed cube starts following.
fn start_distance(host: &dyn ClientHost, cube: [i32; 3]) -> f64 {
    let eye = host
        .local_player()
        .and_then(|player| player.entity().eye_position(host));
    let Some(eye) = eye else {
        return 4.0;
    };
    let dx = eye[0] - (f64::from(cube[0]) + 0.5);
    let dy = eye[1] - (f64::from(cube[1]) + 0.5);
    let dz = eye[2] - (f64::from(cube[2]) + 0.5);
    (dx * dx + dy * dy + dz * dz)
        .sqrt()
        .clamp(MIN_DIST, MAX_DIST)
}

/// The move payload handler: the server keeps the truth and moves the cube.
fn move_apply(cfg: MoveCfg, context: &mut dyn IPayloadContext) -> Result<(), String> {
    let Ok(player) = context.player() else {
        return Ok(());
    };
    let Ok(host) = context.server_host() else {
        return Ok(());
    };
    let uuid = player.uuid();
    match cfg.kind {
        0 => {
            let Some(dim) = player.dimension(&*host).map(|dim| dim.to_string()) else {
                return Ok(());
            };
            let Some(level) = host.level(&dim) else {
                return Ok(());
            };
            let Some(cube) = cube_at(&*host, level, [cfg.x, cfg.y, cfg.z]) else {
                return Ok(());
            };
            let Some(eye) = player.entity().eye_position(&*host) else {
                return Ok(());
            };
            let dx = eye[0] - (f64::from(cube[0]) + 0.5);
            let dy = eye[1] - (f64::from(cube[1]) + 0.5);
            let dz = eye[2] - (f64::from(cube[2]) + 0.5);
            if dx * dx + dy * dy + dz * dz > MOVE_REACH * MOVE_REACH {
                let _ = player.tell(host, Text::literal("异构：离得太远"));
                return Ok(());
            }
            let mut drivers = DRIVERS.lock().unwrap();
            drivers.retain(|(id, _, _)| *id != uuid);
            drivers.push((uuid, dim.clone(), cube));
            drop(drivers);
            // Tell the client where the cube itself is: it clicked a filler, whose owner it
            // cannot read, and needs the cube's own position to keep the follow distance.
            if let Some(reply) = REPLY.get() {
                let truth = Config {
                    kind: 3,
                    x: cube[0],
                    y: cube[1],
                    z: cube[2],
                    dim: dim.clone(),
                    ..Config::default()
                };
                let _ = reply.reply(context, &truth);
            }
        }
        1 => {
            let entry = {
                DRIVERS
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|(id, _, _)| *id == uuid)
                    .cloned()
            };
            let Some((_, dim, mut pos)) = entry else {
                return Ok(());
            };
            let Some(eye) = player.entity().eye_position(&*host) else {
                return Ok(());
            };
            let Some(view) = player.entity().view_vector(&*host) else {
                return Ok(());
            };
            let dist = cfg.dist.clamp(MIN_DIST, MAX_DIST);
            let target = [
                eye[0] + view[0] * dist,
                eye[1] + view[1] * dist,
                eye[2] + view[2] * dist,
            ];
            let raw = [
                target[0].floor() as i32,
                target[1].floor() as i32,
                target[2].floor() as i32,
            ];
            // Never skip a cell: at most one block per axis per packet, so the cube cannot
            // jump over a wall and the swept path is covered by the destination check.
            let cell = [
                raw[0].clamp(pos[0] - 1, pos[0] + 1),
                raw[1].clamp(pos[1] - 1, pos[1] + 1),
                raw[2].clamp(pos[2] - 1, pos[2] + 1),
            ];
            let off = [
                (target[0] - f64::from(cell[0]) - 0.5) as f32,
                (target[1] - f64::from(cell[1]) - 0.5) as f32,
                (target[2] - f64::from(cell[2]) - 0.5) as f32,
            ];
            let mut patch = Compound::new();
            patch.insert("ox".into(), Tag::Float(off[0].to_bits()));
            patch.insert("oy".into(), Tag::Float(off[1].to_bits()));
            patch.insert("oz".into(), Tag::Float(off[2].to_bits()));
            patch.insert("ry".into(), Tag::Int(cfg.yaw));
            patch.insert("rp".into(), Tag::Int(cfg.pitch));
            patch.insert("rr".into(), Tag::Int(cfg.roll));
            let Some(level) = host.level(&dim) else {
                return Ok(());
            };
            if cell != pos {
                if can_move(&*host, level, pos, cell) && relocate(host, level, pos, cell, &patch) {
                    pos = cell;
                    if let Some(slot) = DRIVERS
                        .lock()
                        .unwrap()
                        .iter_mut()
                        .find(|(id, _, _)| *id == uuid)
                    {
                        slot.2 = pos;
                    }
                }
            } else if let Some(entity) = host.block_entity(level, pos) {
                let _ = entity.merge_data(host, patch);
            }
            host.block_entity_sync(pos);
        }
        _ => {
            DRIVERS.lock().unwrap().retain(|(id, _, _)| *id != uuid);
        }
    }
    Ok(())
}

/// Whether one cell of the cube's footprint covers the block cell a given offset away.
fn covers(offset: i32, size: i32) -> bool {
    axis_key(offset, size).is_some()
}

/// Whether the cube at the old cell may slide to the new cell: every cell it would newly
/// cover has to be air, one of its own fillers, or a cell it already covers, so it can never
/// dig into the world.
fn can_move(host: &dyn Host, level: LevelRef, from: [i32; 3], to: [i32; 3]) -> bool {
    let Some(state) = level.block_state(host, from) else {
        return false;
    };
    let size = {
        let meta = host.block_states();
        tenths(state, &meta, "size")
    };
    let mut covered: Vec<[i32; 3]> = Vec::new();
    for dy in -MAX_OFFSET..=MAX_OFFSET {
        for dz in -MAX_OFFSET..=MAX_OFFSET {
            for dx in -MAX_OFFSET..=MAX_OFFSET {
                if covers(dx, size) && covers(dy, size) && covers(dz, size) {
                    covered.push([from[0] + dx, from[1] + dy, from[2] + dz]);
                }
            }
        }
    }
    for dy in -MAX_OFFSET..=MAX_OFFSET {
        for dz in -MAX_OFFSET..=MAX_OFFSET {
            for dx in -MAX_OFFSET..=MAX_OFFSET {
                if !(covers(dx, size) && covers(dy, size) && covers(dz, size)) {
                    continue;
                }
                let at = [to[0] + dx, to[1] + dy, to[2] + dz];
                if covered.contains(&at) {
                    continue;
                }
                let Some(block) = level.block(host, at) else {
                    return false;
                };
                let id = block.id(host);
                if id == AIR || id == BLOCK {
                    continue;
                }
                if id == FILL && owner(host, level, at) == Some(from) {
                    continue;
                }
                return false;
            }
        }
    }
    true
}

/// Moves the cube from `from` to `to`, keeping its data and forcing a filler rebuild.
fn relocate(
    host: &mut dyn ServerHost,
    level: LevelRef,
    from: [i32; 3],
    to: [i32; 3],
    patch: &Compound,
) -> bool {
    let Some(state) = level.block_state(&*host, from) else {
        return false;
    };
    let Some(entity) = host.block_entity(level, from) else {
        return false;
    };
    let Some(data) = entity.data(&*host) else {
        return false;
    };
    let Some(air) = host.block_key(AIR) else {
        return false;
    };
    let air_state = {
        let meta = host.block_states();
        BlockState::of(&meta, air, &[]).ok()
    };
    let Some(air_state) = air_state else {
        return false;
    };
    let flags = BlockUpdateFlags::CLIENTS | BlockUpdateFlags::SUPPRESS_DROPS;
    let _ = level.set_block(host, from, air_state, flags);
    let _ = level.set_block(host, to, state, BlockUpdateFlags::CLIENTS);
    if let Some(moved) = host.block_entity(level, to) {
        let mut merged = data;
        merged.insert("bs".into(), Tag::Int(-1));
        for (key, value) in patch.clone() {
            merged.insert(key, value);
        }
        let _ = moved.merge_data(host, merged);
    }
    clear_fillers(host, level, from);
    host.block_entity_sync(to);
    true
}

/// Removes the filler blocks the cube at `cube` left behind.
fn clear_fillers(host: &mut dyn ServerHost, level: LevelRef, cube: [i32; 3]) {
    let mut doomed: Vec<[i32; 3]> = Vec::new();
    for dx in -MAX_OFFSET..=MAX_OFFSET {
        for dy in -MAX_OFFSET..=MAX_OFFSET {
            for dz in -MAX_OFFSET..=MAX_OFFSET {
                if dx == 0 && dy == 0 && dz == 0 {
                    continue;
                }
                let at = [cube[0] + dx, cube[1] + dy, cube[2] + dz];
                if owner(&*host, level, at) == Some(cube) {
                    doomed.push(at);
                }
            }
        }
    }
    for at in doomed {
        level.remove_block(host, at, false);
    }
}
