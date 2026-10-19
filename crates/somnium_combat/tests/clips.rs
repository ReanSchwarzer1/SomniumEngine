//! A weapon's moves fitted to the animators' clip table (WS3's clips.json
//! format): the hit lands when the clip says, the combo window and dodge
//! cancel follow its events, and moves without a clip keep their timings.
use somnium_combat::clips::ClipTable;
use somnium_combat::weapon;

const TABLE: &str = r#"{
  "notes": "a slice of WS3's table",
  "aliases": {"axe_cut_1": "Axe_Cut_1", "axe_overhead": "Axe_Overhead", "dodge": "Dodge_B"},
  "clips": {
    "Axe_Cut_1": {
      "seconds": 0.633, "loop": false, "travel_m": [0.0, 0.16], "speed_mps": 0.253,
      "foot_down_s": {"l": [0.0, 0.233], "r": [0.0]}, "source": "authored",
      "events": {"hit": [0.24, 0.34], "combo": [0.4, 0.64], "cancel": 0.64},
      "library": "rimeborn_arms",
      "combat_events": {"hit_on": 0.24, "hit_off": 0.34, "recover": 0.64}
    },
    "Axe_Overhead": {
      "seconds": 1.0, "events": {},
      "combat_events": {"hit_on": 0.4, "hit_off": 0.5}
    },
    "Dodge_B": {"seconds": 0.5, "combat_events": {"recover": 0.4}}
  }
}"#;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-4
}

#[test]
fn an_axe_is_fitted_to_its_clips() {
    let table = ClipTable::from_json(TABLE).unwrap();
    let mut axe = weapon::axe();
    let before = axe.clone();
    let fitted = table.retime_weapon(&mut axe);
    assert!(fitted >= 1);
    let cut = axe
        .light
        .iter()
        .find(|m| m.animation == "axe_cut_1")
        .expect("the axe's first cut");
    assert!(near(cut.windup, 0.24) && near(cut.active, 0.10) && near(cut.recovery, 0.30));
    assert!(near(cut.combo_window[0], 0.2) && near(cut.combo_window[1], 1.0));
    assert!(near(cut.dodge_cancel, 1.0));
    // No `recover`: recovery runs to the clip's end; no combo event: the
    // move keeps its own window.
    if let Some(heavy) = axe.heavy.as_ref().filter(|m| m.animation == "axe_overhead") {
        let old = before.heavy.as_ref().unwrap();
        assert!(near(heavy.windup, 0.4) && near(heavy.active, 0.1) && near(heavy.recovery, 0.5));
        assert_eq!(heavy.combo_window, old.combo_window);
    }
    // Moves with no clip in the table are untouched.
    for (now, was) in axe.light.iter().zip(&before.light) {
        if table.clip(&now.animation).is_none() {
            assert_eq!(now, was);
        }
    }
}

#[test]
fn a_warp_lands_the_clips_hit_on_the_designed_frame() {
    let table = ClipTable::from_json(TABLE).unwrap();
    let axe = weapon::axe();
    let cut = &axe.light[0];
    let warp = table.warp(cut).expect("a clip with a hit window");
    // The move's timings are untouched; the clip bends to them.
    assert_eq!(warp.moves, [cut.windup, cut.active, cut.recovery]);
    assert!(near(warp.clip_time(0.0), 0.0));
    assert!(
        near(warp.clip_time(cut.windup), 0.24),
        "hit_on on the designed frame"
    );
    assert!(
        near(warp.clip_time(cut.windup + cut.active), 0.34),
        "hit_off too"
    );
    assert!(
        near(warp.clip_time(10.0), 0.64),
        "held at the end of recovery"
    );
    let [w, a, r] = warp.rates();
    assert!(
        near(w, 0.24 / cut.windup) && near(a, 0.10 / cut.active) && near(r, 0.30 / cut.recovery)
    );
    // The axe's telegraph is slower than its clip: played at 0.57, not stretched.
    assert!(!warp.stretched());
    assert!(
        table.warp(&weapon::sword().light[0]).is_none(),
        "no clip in this slice"
    );
}

#[test]
fn a_clip_without_a_hit_window_changes_nothing() {
    let table = ClipTable::from_json(TABLE).unwrap();
    let mut m = weapon::axe().light[0].clone();
    m.animation = "dodge".into();
    let was = m.clone();
    assert!(!table.retime(&mut m));
    assert_eq!(m, was);
    assert!(ClipTable::from_json("not json").is_err());
}

/// How a real clip table fits every weapon (`SOMNIUM_CLIPS_JSON=<path>
/// cargo test -p somnium_combat --test clips -- --ignored --nocapture`).
#[test]
#[ignore = "reads a game's clip table"]
fn a_real_table_fits_the_armoury() {
    let Ok(path) = std::env::var("SOMNIUM_CLIPS_JSON") else {
        return;
    };
    let table = ClipTable::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    for mut w in [
        weapon::sword(),
        weapon::lance_glaive(),
        weapon::spear(),
        weapon::axe(),
        weapon::knife(),
        weapon::bow(),
        weapon::lattice_rifle(),
        weapon::unarmed(),
        weapon::fangs(),
    ] {
        for m in w
            .light
            .iter()
            .chain(w.heavy.as_ref())
            .chain(w.charge.as_ref())
        {
            if let Some(warp) = table.warp(m) {
                let [a, b, c] = warp.rates();
                let flag = if warp.stretched() {
                    "  << stretched"
                } else {
                    ""
                };
                eprintln!("  warp {}: rates {a:.2} {b:.2} {c:.2}{flag}", m.animation);
            }
        }
        let before = w.clone();
        let fitted = table.retime_weapon(&mut w);
        let moves =
            w.light.len() + usize::from(w.heavy.is_some()) + usize::from(w.charge.is_some());
        eprintln!("{}: {fitted} of {moves} moves fitted", w.name);
        let all = |w: &weapon::WeaponDef| {
            w.light
                .iter()
                .chain(w.heavy.as_ref())
                .chain(w.charge.as_ref())
                .cloned()
                .collect::<Vec<_>>()
        };
        for (now, was) in all(&w).iter().zip(all(&before)) {
            if now != &was {
                eprintln!(
                    "  {}: windup {:.2} -> {:.2}, active {:.2} -> {:.2}, recovery {:.2} -> {:.2}",
                    now.animation,
                    was.windup,
                    now.windup,
                    was.active,
                    now.active,
                    was.recovery,
                    now.recovery
                );
            }
        }
    }
}
