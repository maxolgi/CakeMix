//! Keyed PID mapping tests (multi-session, native, no JS runtime).
//!
//! Multiple WebSRT sessions can reuse the same TS PID numbers; the map
//! is keyed by an opaque u32 (`(sessionId << 16) | pid`, see the
//! `pid_map` field comment in lib.rs) so colliding PIDs coexist. The
//! legacy u16 accessors keep addressing the session-0 entries (key =
//! bare pid).
//!
//! Run: cargo test -p mixer-wasm --test pid_keyed_test

use mixer_wasm::MixerWasm;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;

fn mixer() -> MixerWasm {
    MixerWasm::new(SR, BLOCK, 256).expect("ctor")
}

/// Two sessions sharing the same low 16 bits (both carry PID 0x101) map
/// to disjoint channel ranges; each key reports its own mapping.
#[test]
fn keyed_sessions_same_pid_disjoint_ranges() {
    let mut m = mixer();
    m.map_pid_keyed(0x1_0101, 0, 1).expect("map session 1");
    m.map_pid_keyed(0x2_0101, 4, 2).expect("map session 2");
    assert_eq!(m.pid_channel_keyed(0x1_0101), 0);
    assert_eq!(m.pid_channel_count_keyed(0x1_0101), 1);
    assert_eq!(m.pid_channel_keyed(0x2_0101), 4);
    assert_eq!(m.pid_channel_count_keyed(0x2_0101), 2);
}

/// Unmapping one session's key leaves the other session intact; remove
/// is idempotent (mirror interleave_test's mapping-idempotency style).
#[test]
fn keyed_unmap_leaves_sibling_intact() {
    let mut m = mixer();
    m.map_pid_keyed(0x1_0101, 0, 1).expect("map session 1");
    m.map_pid_keyed(0x2_0101, 4, 2).expect("map session 2");
    m.unmap_pid_keyed(0x2_0101);
    m.unmap_pid_keyed(0x2_0101); // should not panic
    assert_eq!(m.pid_channel_keyed(0x2_0101), -1);
    assert_eq!(m.pid_channel_count_keyed(0x2_0101), 0);
    assert_eq!(m.pid_channel_keyed(0x1_0101), 0);
}

/// Remapping the same key updates the entry (idempotent bookkeeping).
#[test]
fn keyed_remap_updates_entry() {
    let mut m = mixer();
    m.map_pid_keyed(0x1_0101, 0, 1).expect("map");
    m.map_pid_keyed(0x1_0101, 8, 2).expect("remap");
    assert_eq!(m.pid_channel_keyed(0x1_0101), 8);
    assert_eq!(m.pid_channel_count_keyed(0x1_0101), 2);
}

/// u16 back-compat: `map_pid(0x101, …)` and `map_pid_keyed(0x101, …)`
/// address the SAME entry (session 0) through either accessor set.
#[test]
fn u16_accessors_share_session0_namespace() {
    let mut m = mixer();
    m.map_pid(0x101, 0, 2).expect("map via u16");
    assert_eq!(m.pid_channel_keyed(0x101), 0);
    assert_eq!(m.pid_channel_count_keyed(0x101), 2);

    m.map_pid_keyed(0x102, 4, 1).expect("map via keyed");
    assert_eq!(m.pid_channel(0x102), 4);
    assert_eq!(m.pid_channel_count(0x102), 1);

    m.unmap_pid_keyed(0x101);
    assert_eq!(m.pid_channel(0x101), -1);
}
