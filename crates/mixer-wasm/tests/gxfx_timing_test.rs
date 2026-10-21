//! Wall-time budget gate for the gxfx interactive ops (native, release).
//!
//! The E2E FX investigation (2026-10-20) measured every interactive gxfx
//! operation on a live rack to exonerate the engine for the reported
//! "knob drag stalls meters" bug: natively (release) the WORST ops are
//! delay's first enable (1.5 ms — the 2 x 2 MiB tape allocation plus the
//! 1,048,576-float clear) and chorus's create (0.2 ms); param writes are
//! sub-10-µs float stores. The stall itself turned out to be a
//! renderer-level main-thread seizure in the frontend/Chrome layer (it
//! reproduces with every set-fx-* message dropped before the worklet),
//! NOT an engine cost — see README's gxfx section.
//!
//! This gate pins the engine side so a future regression (an accidental
//! allocation, reinit or clear-state on the interactive path) fails CI
//! instead of reaching an E2E run. Budgets carry >= 10x headroom over
//! the measured maxima but are tight enough to catch the pathologies
//! that would actually matter at message time:
//!
//!   op                          measured max   budget
//!   set_slot (create)              0.22 ms      20 ms
//!   set_enabled(true) (tapes)      1.47 ms      50 ms
//!   set_param + one block          0.03 ms       1 ms
//!   process_pair block (chain on)  0.05 ms     500 µs (FX pass alone)
//!   disable + enable churn pair    0.14 ms      50 ms
//!   teardown (destroy)             0.04 ms      20 ms
//!
//! Timing assertions are release-only (glitch_sim's `require_release`
//! pattern): debug builds skip the budgets but still exercise the ops.
//!
//! Run: cargo test -p mixer-wasm --release --test gxfx_timing_test -- --nocapture

use std::time::Instant;

use mixer_wasm::gxfx::GxFxRack;

const FS: u32 = 48_000;
const BS: usize = 128;

/// Real-time-ish budget helper: debug builds skip (see module docs).
#[cfg(debug_assertions)]
fn require_release() -> bool {
    eprintln!("gxfx_timing_test: debug build — timing budgets skipped (run with --release)");
    false
}
#[cfg(not(debug_assertions))]
fn require_release() -> bool {
    true
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

#[test]
fn interactive_ops_within_wall_time_budget() {
    let release = require_release();
    let mut r = GxFxRack::new(FS, BS);
    let mut l = vec![0.1f32; BS];
    let mut rr = vec![0.1f32; BS];

    for fx in 0..11i16 {
        let name = [
            "wah",
            "overdrive",
            "distortion",
            "compressor",
            "chorus",
            "flanger",
            "phaser",
            "tremolo",
            "delay",
            "echo",
            "reverb",
        ][fx as usize];

        // -- set_slot (eager create; set_samplerate + port connects) --
        let t = Instant::now();
        r.set_slot(0, 0, fx).unwrap();
        let set_slot_ms = ms(t.elapsed());

        // -- enable (first activate: memory-class effects new their tapes
        //    here and zero them) --
        let t = Instant::now();
        r.set_enabled(0, 0, true).unwrap();
        let enable_ms = ms(t.elapsed());

        // -- steady per-block process cost with the chain enabled --
        let mut worst_block_us = 0.0f64;
        let t = Instant::now();
        for _ in 0..200 {
            let b = Instant::now();
            r.process_pair(0, &mut l, Some(&mut rr));
            worst_block_us = worst_block_us.max(b.elapsed().as_secs_f64() * 1e6);
        }
        let block_avg_us = t.elapsed().as_secs_f64() * 1e6 / 200.0;

        // -- param writes: every live param, each followed by a block
        //    (the E2E drag shape: value changes interleaved with quanta) --
        let live_params = (0..48)
            .find(|p| r.set_param(0, 0, *p, 100.0).is_err())
            .unwrap_or(0);
        let rounds = 200u32;
        let t = Instant::now();
        for k in 0..rounds {
            for p in 0..live_params {
                r.set_param(0, 0, p, 100.0 + k as f32).unwrap();
                r.process_pair(0, &mut l, Some(&mut rr));
            }
        }
        let per_write_ms = ms(t.elapsed()) / (rounds as usize * live_params.max(1)) as f64;

        // -- enable/disable churn: tape free/realloc cycles (bypass
        //    toggling) --
        let cycles = 20u32;
        let t = Instant::now();
        for _ in 0..cycles {
            r.set_enabled(0, 0, false).unwrap();
            r.set_enabled(0, 0, true).unwrap();
        }
        let churn_pair_ms = ms(t.elapsed()) / cycles as f64;

        // -- teardown (deactivate + delete + free) --
        let t = Instant::now();
        r.set_slot(0, 0, -1).unwrap();
        let teardown_ms = ms(t.elapsed());

        println!(
            "fx {name:11}: set_slot={set_slot_ms:7.3} ms  enable={enable_ms:7.3} ms  block_avg={block_avg_us:7.2} us  worst_block={worst_block_us:7.2} us  param+block={per_write_ms:7.4} ms  dis+en={churn_pair_ms:7.3} ms  teardown={teardown_ms:7.3} ms  (params={live_params})"
        );

        if release {
            assert!(
                set_slot_ms <= 20.0,
                "fx {name}: set_slot took {set_slot_ms:.3} ms (budget 20 ms) — eager create must stay one-time-cheap at message time"
            );
            assert!(
                enable_ms <= 50.0,
                "fx {name}: set_enabled(true) took {enable_ms:.3} ms (budget 50 ms) — first-activate tape allocation must stay off the RT path"
            );
            assert!(
                per_write_ms <= 1.0,
                "fx {name}: param write + block took {per_write_ms:.4} ms (budget 1 ms) — param writes must remain float stores"
            );
            assert!(
                worst_block_us <= 500.0,
                "fx {name}: process_pair block took {worst_block_us:.1} us (budget 500 us for the FX pass alone, RT block budget is 2667 us)"
            );
            assert!(
                churn_pair_ms <= 50.0,
                "fx {name}: disable+enable churn pair took {churn_pair_ms:.3} ms (budget 50 ms) — bypass toggling must not degrade"
            );
            assert!(
                teardown_ms <= 20.0,
                "fx {name}: teardown took {teardown_ms:.3} ms (budget 20 ms)"
            );
        }
    }
}
