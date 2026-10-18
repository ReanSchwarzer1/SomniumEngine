//! The runtime end to end: conversations, quests, barks, the journal and saves, over the shared test content.

use super::*;
use crate::conversation::{Beat, Conversation, TranscriptEntry, simulate};
use crate::database::{StoryDatabase, StorySources, tests::sources};
use crate::quest_log::{ObjectiveStatus, QuestStatus};

/// The test content plus a few dialogues that exercise the runtime's corners.
fn content() -> StorySources {
    let mut sources = sources();
    sources.add_file(
        "dialogue/foreman.somdialogue",
        r#"{
        "version": 1, "id": "foreman", "on_interrupt": "hey",
        "entries": {"again": "back"},
        "nodes": [
            {"type": "line", "id": "n1", "speaker": "foreman", "text": "You the lockpick?",
             "effects": "met.foreman = true; start_quest('varga_job')", "next": ["hub"]},
            {"type": "jump", "id": "hub", "target": "menu"},
            {"type": "line", "id": "menu", "speaker": "foreman", "text": "Well?", "next": ["ask_pay", "ask_guards", "mend", "bluff", "leave"]},
            {"type": "option", "id": "ask_pay", "text": "What's it pay?", "once": true, "effects": "gold += 50", "next": ["pay"]},
            {"type": "line", "id": "pay", "speaker": "foreman", "text": "Fifty now. You have {gold} crowns.", "next": ["menu"]},
            {"type": "option", "id": "ask_guards", "text": "How many guards?", "next": ["guards"]},
            {"type": "line", "id": "guards", "speaker": "foreman", "text": "Three.", "next": ["menu"]},
            {"type": "option", "id": "mend", "text": "Your hand is broken.",
             "check": {"category": "skill", "fact": "skill.mending", "at_least": 3}, "next": ["thanks"]},
            {"type": "line", "id": "thanks", "speaker": "foreman", "text": "...Thank you.", "effects": "rep.varga += 1", "next": ["menu"]},
            {"type": "option", "id": "bluff", "text": "I've done this a hundred times.",
             "check": {"category": "skill", "fact": "skill.mending", "roll": {"sides": 20, "difficulty": 11}},
             "next": ["bluff_ok"], "fail": ["bluff_no"]},
            {"type": "line", "id": "bluff_ok", "speaker": "foreman", "text": "Good.", "next": ["menu"]},
            {"type": "line", "id": "bluff_no", "speaker": "foreman", "text": "Liar.", "next": ["menu"]},
            {"type": "option", "id": "leave", "text": "Later.", "next": ["bye"]},
            {"type": "end", "id": "bye", "result": "left"},
            {"type": "line", "id": "back", "speaker": "foreman", "text": "Back again?", "next": ["menu"]},
            {"type": "line", "id": "hey", "speaker": "foreman", "text": "Oi, I'm talking to you!"}
        ]}"#,
    );
    sources.add_file(
        "dialogue/caller.somdialogue",
        r#"{
        "version": 1, "id": "caller",
        "nodes": [
            {"type": "line", "id": "n1", "speaker": "caelan", "text": "Before we go...", "next": ["c1"]},
            {"type": "call", "id": "c1", "dialogue": "aside", "next": ["n2"]},
            {"type": "line", "id": "n2", "speaker": "caelan", "text": "Where was I?", "next": ["a1"]},
            {"type": "action", "id": "a1", "effects": "camera('wide')", "wait": true, "next": ["j1"]},
            {"type": "jump", "id": "j1", "target": "aside/n2"}
        ]}"#,
    );
    sources.add_file(
        "dialogue/aside.somdialogue",
        r#"{
        "version": 1, "id": "aside",
        "nodes": [
            {"type": "line", "id": "n1", "speaker": "mireille", "text": "A word.", "next": ["e1"]},
            {"type": "end", "id": "e1", "result": "aside done"},
            {"type": "line", "id": "n2", "speaker": "mireille", "text": "You were saying nothing.", "next": ["e2"]},
            {"type": "end", "id": "e2", "result": "jumped"}
        ]}"#,
    );
    sources
}

fn story() -> Story {
    let (database, warnings) = StoryDatabase::build(content()).unwrap_or_else(|issues| {
        panic!(
            "{}",
            issues
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        )
    });
    assert!(warnings.is_empty(), "{warnings:#?}");
    Story::new(Arc::new(database), 7)
}

fn line(conversation: &Conversation) -> &crate::conversation::LineBeat {
    match conversation.beat() {
        Beat::Line(line) => line,
        other => panic!("expected a line, got {other:?}"),
    }
}

fn choices(conversation: &Conversation) -> Vec<(String, bool)> {
    match conversation.beat() {
        Beat::Choices { choices, .. } => choices
            .iter()
            .map(|c| (c.node.clone(), c.available))
            .collect(),
        other => panic!("expected choices, got {other:?}"),
    }
}

#[test]
fn a_conversation_plays_and_its_effects_land_as_lines_show() {
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "mireille_intro", "").unwrap();
    let first = line(&talk);
    assert_eq!(
        (first.name.as_str(), first.expression.as_str()),
        ("Mireille", "smile")
    );
    assert_eq!(first.color, Some([0xc8, 0xa2, 0xff, 255]));
    assert!(first.menu_next && !first.seen);
    assert_eq!(
        story.fact("met.mireille"),
        Some(Value::Bool(true)),
        "set as the line shows"
    );
    talk.advance(&mut story).unwrap();
    assert_eq!(
        choices(&talk),
        [("o1".to_owned(), true), ("o2".to_owned(), false)],
        "the language check shows locked; the timeout option is hidden"
    );
    let Beat::Choices {
        choices: shown,
        timeout_ms,
        ..
    } = talk.beat().clone()
    else {
        panic!()
    };
    assert_eq!(timeout_ms, Some(8000));
    let check = shown[1].check.as_ref().unwrap();
    assert_eq!(
        (check.label.as_str(), check.passes),
        ("Patter: conversant", Some(false))
    );
    assert!(
        talk.choose(&mut story, 1).is_err(),
        "a locked choice cannot be taken"
    );
    talk.choose(&mut story, 0).unwrap();
    assert_eq!(line(&talk).node, "n2");
    talk.advance(&mut story).unwrap();
    assert_eq!(
        talk.beat(),
        &Beat::End {
            result: "met".into()
        },
        "affection 1 is under the branch's 2"
    );
    assert_eq!(story.fact("mireille.affection"), Some(Value::Int(1)));
    assert_eq!(story.visits("mireille_intro", "o1"), 1);
}

#[test]
fn passing_a_language_check_opens_the_other_road() {
    let mut story = story();
    story.set("lang.patter", "fluent").unwrap();
    let transcript = simulate(&mut story, "mireille_intro", "", &[1], 32).unwrap();
    let lines: Vec<&str> = transcript
        .iter()
        .filter_map(|e| match e {
            TranscriptEntry::Line { node, .. } => Some(node.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, ["n1", "n3", "n4"]);
    assert_eq!(
        transcript.last(),
        Some(&TranscriptEntry::End {
            result: "met".into()
        })
    );
}

#[test]
fn a_timed_menu_takes_its_silence_when_time_runs_out() {
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "mireille_intro", "").unwrap();
    talk.advance(&mut story).unwrap();
    assert!(!talk.tick(&mut story, 7_999));
    assert_eq!(talk.time_left_ms(), Some(1));
    assert!(talk.tick(&mut story, 1));
    assert_eq!(line(&talk).node, "n2", "the silent option leads to n2");
    assert_eq!(story.visits("mireille_intro", "o3"), 1);
}

#[test]
fn hubs_loop_once_options_vanish_and_gated_checks_unlock() {
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "foreman", "").unwrap();
    assert_eq!(
        story.quest_status("varga_job"),
        QuestStatus::Active,
        "started from the line's effects"
    );
    talk.advance(&mut story).unwrap();
    assert_eq!(line(&talk).node, "menu", "through the jump");
    talk.advance(&mut story).unwrap();
    let menu = choices(&talk);
    assert_eq!(menu.len(), 5);
    assert_eq!(
        menu[2],
        ("mend".to_owned(), false),
        "Mending 0 of 3: shown locked"
    );
    talk.choose(&mut story, 0).unwrap();
    assert_eq!(line(&talk).text, "Fifty now. You have 50 crowns.");
    talk.advance(&mut story).unwrap();
    talk.advance(&mut story).unwrap();
    assert_eq!(choices(&talk).len(), 4, "the once-only option is gone");
    story.set("skill.mending", 4).unwrap();
    talk.advance(&mut story).ok();
    // The menu does not re-evaluate by itself; the next visit to it does.
    talk.choose(&mut story, 0).unwrap();
    talk.advance(&mut story).unwrap();
    talk.advance(&mut story).unwrap();
    assert_eq!(
        choices(&talk)[1],
        ("mend".to_owned(), true),
        "Mending 4 unlocks it"
    );
    assert!(line_seen_after_loop(&mut story));
}

fn line_seen_after_loop(story: &mut Story) -> bool {
    let mut talk = Conversation::start(story, "foreman", "again").unwrap();
    assert_eq!(line(&talk).node, "back", "a named entry");
    talk.advance(story).unwrap();
    line(&talk).seen
}

#[test]
fn rolls_are_seeded_and_their_odds_are_shown() {
    let play = |seed: u64| {
        let (database, _) = StoryDatabase::build(content()).unwrap();
        let mut story = Story::new(Arc::new(database), seed);
        story.set("skill.mending", 5).unwrap();
        let mut talk = Conversation::start(&mut story, "foreman", "").unwrap();
        talk.advance(&mut story).unwrap();
        talk.advance(&mut story).unwrap();
        let Beat::Choices { choices, .. } = talk.beat().clone() else {
            panic!()
        };
        let bluff = choices.iter().find(|c| c.node == "bluff").unwrap();
        assert_eq!(
            bluff.check.as_ref().unwrap().chance,
            Some(75),
            "5 + d20 >= 11: 15 of 20 faces"
        );
        talk.choose(&mut story, bluff.index).unwrap();
        (
            line(&talk).node.clone(),
            talk.last_check().cloned().unwrap(),
        )
    };
    let (node, check) = play(7);
    assert_eq!(
        play(7),
        (node.clone(), check.clone()),
        "the same seed rolls the same"
    );
    let (die, total) = check.roll.unwrap();
    assert_eq!(total, die + 5);
    assert_eq!(check.passed, total >= 11);
    assert_eq!(node, if check.passed { "bluff_ok" } else { "bluff_no" });
}

#[test]
fn calls_return_waits_hold_and_jumps_cross_dialogues() {
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "caller", "").unwrap();
    talk.advance(&mut story).unwrap();
    assert_eq!(
        (line(&talk).dialogue.as_str(), line(&talk).node.as_str()),
        ("aside", "n1")
    );
    talk.advance(&mut story).unwrap();
    assert_eq!(
        line(&talk).node,
        "n2",
        "the called dialogue's end returned to the caller"
    );
    assert_eq!(talk.dialogue(), "caller");
    talk.advance(&mut story).unwrap();
    assert!(matches!(talk.beat(), Beat::Wait { node, .. } if node == "a1"));
    assert!(story.drain_events().iter().any(
        |e| matches!(e, StoryEvent::Command { name, args } if name == "camera" && args == &[Value::Text("wide".into())])
    ));
    assert!(
        talk.advance(&mut story).is_err(),
        "a wait is resumed, not advanced"
    );
    talk.resume(&mut story).unwrap();
    assert_eq!(
        (line(&talk).dialogue.as_str(), line(&talk).node.as_str()),
        ("aside", "n2")
    );
    talk.advance(&mut story).unwrap();
    assert_eq!(
        talk.beat(),
        &Beat::End {
            result: "jumped".into()
        }
    );
}

#[test]
fn an_interruption_plays_its_handler_once() {
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "foreman", "").unwrap();
    assert!(talk.interrupt(&mut story));
    assert_eq!(line(&talk).node, "hey");
    assert!(!talk.interrupt(&mut story), "a second interruption ends it");
    assert_eq!(
        talk.beat(),
        &Beat::End {
            result: "interrupted".into()
        }
    );
    let mut talk = Conversation::start(&mut story, "foreman", "").unwrap();
    talk.interrupt(&mut story);
    talk.advance(&mut story).unwrap();
    assert_eq!(
        talk.beat(),
        &Beat::End {
            result: "interrupted".into()
        },
        "after the handler line"
    );
    let mut quiet = Conversation::start(&mut story, "mireille_intro", "").unwrap();
    assert!(!quiet.interrupt(&mut story), "no handler: it just ends");
}

#[test]
fn a_quest_runs_its_stages_objectives_and_branches() {
    let mut story = story();
    story.drain_events();
    story.start_quest("varga_job").unwrap();
    let events = story.drain_events();
    assert_eq!(
        events[..3],
        [
            StoryEvent::QuestStarted {
                quest: "varga_job".into()
            },
            StoryEvent::QuestStage {
                quest: "varga_job".into(),
                stage: "offer".into()
            },
            StoryEvent::JournalAdded { sequence: 0 },
        ]
    );
    story.set("met.foreman", true).unwrap();
    assert_eq!(story.quests().state("varga_job").unwrap().stage, "inside");
    story.track(Some("varga_job"));
    let markers = story.markers();
    assert_eq!(markers.len(), 2);
    assert!(markers.iter().all(|m| m.tracked));
    assert_eq!(markers[0].anchor, "varga_quay");
    story.set("varga.guards_down", 2).unwrap();
    let views = story.objectives("varga_job");
    let guards = views.iter().find(|v| v.id == "guards").unwrap();
    assert_eq!(
        (guards.progress, guards.status),
        (Some((2, 3)), ObjectiveStatus::Active)
    );
    story.run("varga.guards_down += 1").unwrap();
    assert_eq!(
        story
            .quests()
            .state("varga_job")
            .unwrap()
            .objective("guards"),
        Some(ObjectiveStatus::Done)
    );
    story.set("varga.alarm", true).unwrap();
    assert_eq!(
        story
            .quests()
            .state("varga_job")
            .unwrap()
            .objective("unseen"),
        Some(ObjectiveStatus::Failed),
        "an optional objective fails without failing the quest"
    );
    assert_eq!(story.quest_status("varga_job"), QuestStatus::Active);
    assert_eq!(story.reached("strongroom_door"), 1);
    assert_eq!(story.quest_status("varga_job"), QuestStatus::Completed);
    assert_eq!(
        story.quests().state("varga_job").unwrap().path,
        ["offer", "inside", "loud"]
    );
    assert_eq!(story.fact("rep.varga"), Some(Value::Int(5)), "on_complete");
    assert_eq!(
        story.quests().tracked,
        None,
        "a finished quest is no longer tracked"
    );
    let view = story.journal_view();
    let page = &view.quests[0];
    assert_eq!(page.title, "The Varga Strongroom");
    assert_eq!(page.giver, "Varga Foreman");
    let tiers: Vec<(&str, Option<usize>)> = page
        .lines
        .iter()
        .map(|l| (l.tier.as_str(), l.tier_rank))
        .collect();
    assert_eq!(tiers, [("witnessed", Some(2)), ("retold", Some(4))]);
}

#[test]
fn arrival_is_measured_in_game_metres_around_anchors_and_regions() {
    let mut sources = content();
    sources.add_file(
        "quests/walk.somquest",
        r#"{"version": 1, "id": "walk", "title": "Walk", "stages": [
            {"id": "go", "objectives": [
                {"id": "mere", "text": "Walk to the mere", "marker": {"position": [40.0, 0.0, -10.0], "region": "eirvane", "radius": 5.0, "arrive": true}},
                {"id": "gate", "text": "Reach the gate", "marker": {"anchor": "hall_gate", "radius": 3.0, "arrive": true}}],
             "next": [{"to": "done"}]},
            {"id": "done", "end": "complete"}]}"#,
    );
    let (database, _) = StoryDatabase::build(sources).unwrap();
    let mut story = Story::new(Arc::new(database), 1);
    story.start_quest("walk").unwrap();
    let anchor = |name: &str| (name == "hall_gate").then_some([10_620.0, 0.0, 6_790.0]);
    let region = |name: &str| (name == "eirvane").then_some([10_600.0, 0.0, 6_800.0]);
    let markers = story.markers();
    assert_eq!(markers[0].region, "eirvane");
    assert_eq!(
        story.arrived_at([10_600.0, 0.0, 6_800.0], anchor, region),
        0,
        "the anchor itself is not the mere"
    );
    assert_eq!(
        story.arrived_at([10_643.0, 0.0, 6_791.0], anchor, region),
        1,
        "40 m east, 10 m north of the anchor"
    );
    assert_eq!(
        story.arrived_at([10_621.0, 0.0, 6_791.0], anchor, region),
        1
    );
    assert_eq!(story.quest_status("walk"), QuestStatus::Completed);
}

#[test]
fn a_running_timer_fails_a_required_objective_and_the_quest() {
    let mut story = story();
    story.start_quest("varga_job").unwrap();
    story.set("met.foreman", true).unwrap();
    story.tick(59_999);
    assert_eq!(story.quest_status("varga_job"), QuestStatus::Active);
    let remaining = story
        .objectives("varga_job")
        .into_iter()
        .find(|v| v.id == "door")
        .unwrap()
        .remaining_ms;
    assert_eq!(remaining, Some(1));
    story.drain_events();
    story.tick(1);
    assert_eq!(story.quest_status("varga_job"), QuestStatus::Failed);
    assert_eq!(
        story.drain_events(),
        [
            StoryEvent::ObjectiveFailed {
                quest: "varga_job".into(),
                objective: "door".into()
            },
            StoryEvent::QuestFailed {
                quest: "varga_job".into()
            },
        ]
    );
}

#[test]
fn saves_round_trip_through_the_game_state_and_replay_identically() {
    let mut story = story();
    story.start_quest("varga_job").unwrap();
    story.set("met.foreman", true).unwrap();
    story.run("journal('earth_mother'); gold = 12").unwrap();
    story.tick(5_000);
    let save = story.save();
    let mut game_state = serde_json::Value::Null;
    save.write_into(&mut game_state).unwrap();
    let text = serde_json::to_string(&game_state).unwrap();
    let back = StorySave::read_from(&serde_json::from_str(&text).unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(back, save);
    let (mut loaded, notes) = Story::load(Arc::clone(story.database()), &back).unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(loaded.save(), save);
    for s in [&mut story, &mut loaded] {
        s.drain_events();
        s.set("varga.guards_down", 3).unwrap();
        s.tick(55_000);
    }
    assert_eq!(story.drain_events(), loaded.drain_events());
    assert_eq!(
        serde_json::to_string(&story.save()).unwrap(),
        serde_json::to_string(&loaded.save()).unwrap()
    );
    let mut other = save.clone();
    other.story = "another_game".into();
    assert!(Story::load(Arc::clone(story.database()), &other).is_err());
    assert_eq!(
        StorySave::read_from(&serde_json::json!({"inventory": []})).unwrap(),
        None
    );
}

/// Save `story` and `talk` through a game's save state as text, and load both back.
fn reload(story: &Story, talk: &Conversation) -> (Story, Conversation) {
    let mut state = serde_json::Value::Null;
    story.save().write_into(&mut state).unwrap();
    talk.save().unwrap().write_into(&mut state).unwrap();
    let state: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    let (mut loaded, notes) = Story::load(
        Arc::clone(story.database()),
        &StorySave::read_from(&state).unwrap().unwrap(),
    )
    .unwrap();
    assert!(notes.is_empty(), "{notes:?}");
    let saved = crate::ConversationSave::read_from(&state).unwrap().unwrap();
    let talk = Conversation::restore(&mut loaded, &saved).unwrap();
    (loaded, talk)
}

#[test]
fn a_scene_saved_mid_menu_or_mid_call_reloads_showing_and_going_on_the_same() {
    // In a menu, after the menu's line ran its effects (a quest started) and a once-choice was taken.
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "foreman", "").unwrap();
    for _ in 0..2 {
        talk.advance(&mut story).unwrap();
    }
    talk.choose(&mut story, 0).unwrap();
    for _ in 0..2 {
        talk.advance(&mut story).unwrap();
    }
    assert!(!choices(&talk).iter().any(|(id, _)| id == "ask_pay"));
    story.drain_events();
    let (mut loaded, mut again) = reload(&story, &talk);
    assert_eq!(
        again.beat(),
        talk.beat(),
        "the same menu, the once-choice still gone"
    );
    assert_eq!(
        again.current_line(&loaded).map(|l| l.text),
        Some("Well?".to_owned()),
        "the line the menu stands under, for the presenter"
    );
    assert!(
        loaded.drain_events().is_empty(),
        "nothing ran again on load"
    );
    for (s, t) in [(&mut story, &mut talk), (&mut loaded, &mut again)] {
        let bluff = choices(t).iter().position(|(id, _)| id == "bluff").unwrap();
        t.choose(s, bluff).unwrap();
    }
    assert_eq!(again.beat(), talk.beat(), "the seeded roll lands the same");
    assert_eq!(again.last_check(), talk.last_check());

    // Inside a called dialogue, then at a wait.
    let mut story = self::story();
    let mut talk = Conversation::start(&mut story, "caller", "").unwrap();
    talk.advance(&mut story).unwrap();
    let (mut loaded, mut again) = reload(&story, &talk);
    assert_eq!(
        (line(&again).dialogue.as_str(), line(&again).node.as_str()),
        ("aside", "n1")
    );
    for (s, t) in [(&mut story, &mut talk), (&mut loaded, &mut again)] {
        t.advance(s).unwrap();
        t.advance(s).unwrap();
    }
    assert!(
        matches!(again.beat(), Beat::Wait { node, .. } if node == "a1"),
        "the call returned"
    );
    let (mut loaded, mut again) = reload(&loaded, &again);
    again.resume(&mut loaded).unwrap();
    talk.resume(&mut story).unwrap();
    assert_eq!(again.beat(), talk.beat());

    // A finished scene saves nothing; a scene whose node was cut does not load, and says so.
    talk.advance(&mut story).unwrap();
    assert!(talk.is_over() && talk.save().is_none());
    let mut cut = again.save().unwrap();
    cut.node = "gone".into();
    assert!(Conversation::restore(&mut loaded, &cut).is_err());
}

#[test]
fn the_presenter_log_saves_without_the_line_on_screen() {
    let mut story = story();
    let mut talk = Conversation::start(&mut story, "caller", "").unwrap();
    let mut presenter = crate::Presenter::default();
    presenter.show_line(line(&talk).clone(), None);
    talk.advance(&mut story).unwrap();
    presenter.show_line(line(&talk).clone(), None);
    let saved = presenter.history_for_save();
    assert_eq!(
        saved.len(),
        1,
        "the line on screen is shown again on load, not saved twice"
    );
    let mut back = crate::Presenter::default();
    back.restore_history(saved);
    back.show_line(line(&talk).clone(), None);
    assert_eq!(back.history(), presenter.history());
}

fn with_voice() -> Story {
    let mut sources = content();
    sources.add_file(
        "notes/voice.somnotes",
        r#"{"version": 1, "id": "voice", "voice": [
            {"id": "purse", "category": "skill", "heading": "PURSE COUNTED",
             "text": "Confirmed. You hold {gold} crowns."}]}"#,
    );
    let (database, _) = StoryDatabase::build(sources).unwrap_or_else(|i| panic!("{i:?}"));
    Story::new(Arc::new(database), 7)
}

#[test]
fn the_voice_logs_what_it_said_as_it_said_it_and_the_log_saves() {
    let mut story = with_voice();
    story.run("gold = 12; voice('purse')").unwrap();
    let events = story.drain_events();
    let Some(StoryEvent::Voice { sequence, line }) = events
        .iter()
        .find(|e| matches!(e, StoryEvent::Voice { .. }))
    else {
        panic!("{events:?}")
    };
    assert_eq!(line, "purse");
    let said = story.voice_view(*sequence).unwrap();
    assert_eq!(
        (said.heading.as_str(), said.text.as_str()),
        ("PURSE COUNTED", "Confirmed. You hold 12 crowns.")
    );
    story.set("gold", 40).unwrap();
    assert_eq!(
        story.voice_view(*sequence).unwrap().text,
        "Confirmed. You hold 12 crowns.",
        "the log reads as it was said"
    );
    story.voice("purse").unwrap();
    let view = story.journal_view();
    assert_eq!(
        view.voice
            .iter()
            .map(|v| v.text.as_str())
            .collect::<Vec<_>>(),
        [
            "Confirmed. You hold 12 crowns.",
            "Confirmed. You hold 40 crowns."
        ],
        "said twice, logged twice, oldest first"
    );
    assert!(view.notes.is_empty() && view.voice.iter().all(|v| !v.read));
    let (loaded, _) = Story::load(Arc::clone(story.database()), &story.save()).unwrap();
    assert_eq!(loaded.journal_view(), view);
    assert!(story.voice("nobody").is_err());

    let mut sources = content();
    sources.add_file(
        "dialogue/bad.somdialogue",
        r#"{"version": 1, "id": "bad", "nodes": [{"type": "action", "id": "a", "effects": "voice('nope')"}]}"#,
    );
    let issues = StoryDatabase::build(sources).unwrap_err();
    assert!(
        issues
            .iter()
            .any(|i| i.message.contains("no such voice line")),
        "{issues:?}"
    );
}

#[test]
fn people_join_the_journal_when_heard_or_written_about() {
    let mut story = story();
    assert!(story.journal_view().people.is_empty());
    let mut talk = Conversation::start(&mut story, "foreman", "").unwrap();
    talk.advance(&mut story).unwrap();
    let people = story.journal_view().people;
    assert_eq!(people.len(), 1, "{people:?}");
    assert_eq!(
        (people[0].id.as_str(), people[0].lines_heard),
        ("foreman", 2)
    );
    assert!(!people[0].name.is_empty());
}

fn aldermere() -> Story {
    let mut sources = content();
    sources.add_file(
        "barks/aldermere.sombarks",
        r#"{"version": 1, "id": "aldermere",
        "barks": [
            {"id": "morning", "trigger": "greet", "text": "Morning.", "places": ["aldermere"], "hours": [5, 11]},
            {"id": "evening", "trigger": "greet", "text": "Evening.", "places": ["aldermere"], "hours": [17, 22]},
            {"id": "late", "trigger": "greet", "text": "Off to bed, you.", "places": ["aldermere"], "hours": [22, 4]},
            {"id": "lidfall", "trigger": "greet", "text": "The Lid's falling. Inside!", "eclipse": [0.8, 1.0], "priority": 5},
            {"id": "hello_a", "trigger": "hello", "text": "Hello."},
            {"id": "hello_b", "trigger": "hello", "text": "Hullo there."},
            {"id": "alarm", "trigger": "thief", "text": "Thief! Stop!", "priority": 10}
        ],
        "chatter": [
            {"id": "weather", "trigger": "murmur", "places": ["aldermere"],
             "lines": [{"role": "a", "text": "Cold one."}, {"role": "b", "text": "Colder tomorrow."}]}
        ]}"#,
    );
    let (database, _) = StoryDatabase::build(sources).unwrap_or_else(|i| panic!("{i:?}"));
    Story::new(Arc::new(database), 11)
}

#[test]
fn barks_follow_the_place_the_hour_and_the_lid() {
    use crate::barks::BarkContext;
    let mut story = aldermere();
    let near = ["anna"];
    let at = |place, hour: Option<f32>, eclipse| BarkContext {
        place,
        hour,
        eclipse,
        ..BarkContext::near(&near)
    };
    let id =
        |story: &mut Story, context: BarkContext| story.bark_in("greet", &context).map(|b| b.id);
    assert_eq!(
        id(&mut story, at("aldermere/market", Some(8.0), 0.0)).as_deref(),
        Some("morning")
    );
    assert_eq!(
        id(&mut story, at("aldermere", Some(18.5), 0.0)).as_deref(),
        Some("evening")
    );
    assert_eq!(
        id(&mut story, at("aldermere", Some(2.0), 0.0)).as_deref(),
        Some("late"),
        "past midnight"
    );
    assert_eq!(
        id(&mut story, at("aldermere_west", Some(8.0), 0.0)),
        None,
        "a name is not a place under it"
    );
    assert_eq!(id(&mut story, at("eirvane_hall", Some(8.0), 0.0)), None);
    assert_eq!(
        id(&mut story, at("aldermere", None, 0.0)),
        None,
        "no hour, no hourly bark"
    );
    assert_eq!(
        id(&mut story, at("eirvane_hall", Some(8.0), 0.9)).as_deref(),
        Some("lidfall")
    );
    assert_eq!(
        story.bark("greet", &near),
        None,
        "the old call asks for nowhere, at no hour, under a clear sky"
    );

    let mut sources = content();
    sources.add_file(
        "barks/bad.sombarks",
        r#"{"version": 1, "id": "bad", "barks": [
            {"id": "b", "trigger": "t", "text": "x", "hours": [3, 25], "eclipse": [0.9, 0.2], "places": ["a//b"]}]}"#,
    );
    let issues = StoryDatabase::build(sources).unwrap_err();
    for wanted in ["hours", "eclipse", "places"] {
        assert!(
            issues.iter().any(|i| i.message.starts_with(wanted)),
            "{wanted}: {issues:?}"
        );
    }
}

#[test]
fn the_ambience_keeps_one_voice_puts_barks_over_chatter_and_keeps_lines_fresh() {
    use crate::ambience::{Ambience, AmbienceSettings, Heard};
    use crate::barks::BarkContext;
    let mut story = aldermere();
    let mut ambience = Ambience::new(AmbienceSettings::default());
    let near = ["anna", "bram", "cara"];
    let here = BarkContext {
        place: "aldermere/market",
        hour: Some(9.0),
        ..BarkContext::near(&near)
    };
    assert!(
        ambience.idle(&mut story, "murmur", &here).is_none(),
        "too soon after arriving"
    );
    story.tick(20_000);
    let chat = ambience.idle(&mut story, "murmur", &here).unwrap();
    let Heard::Chatter { cast, .. } = &chat.heard else {
        panic!("{chat:?}")
    };
    assert_eq!(cast, &["anna", "bram"]);
    assert!(ambience.busy(story.clock_ms()));

    let hello = ambience.event(&mut story, "hello", &here).unwrap();
    assert!(hello.interrupts, "a bark cuts chatter off");
    let Heard::Bark(first) = &hello.heard else {
        panic!()
    };
    assert_eq!(first.speaker, "cara", "the two chatting are resting");
    assert!(
        ambience.event(&mut story, "hello", &here).is_none(),
        "a bark does not cut off a bark as plain"
    );
    let alarm = ambience.event(&mut story, "thief", &here).unwrap();
    let Heard::Bark(alarm_pick) = &alarm.heard else {
        panic!()
    };
    assert!(
        alarm.interrupts && alarm_pick.id == "alarm",
        "an urgent one does, rested or not"
    );

    story.tick(30_000);
    let again = |ambience: &mut Ambience, story: &mut Story| {
        story.tick(10_000);
        match ambience.event(story, "hello", &here).map(|s| s.heard) {
            Some(Heard::Bark(pick)) => pick.id,
            other => panic!("{other:?}"),
        }
    };
    let (one, two, three) = (
        again(&mut ambience, &mut story),
        again(&mut ambience, &mut story),
        again(&mut ambience, &mut story),
    );
    assert_ne!(one, two, "the line just heard waits while another fits");
    assert_ne!(two, three);
    ambience.finished(&story);
    assert!(!ambience.busy(story.clock_ms()));
    assert!(
        ambience.idle(&mut story, "murmur", &here).is_none(),
        "chatter waits for a quiet stretch"
    );
}

#[test]
fn a_save_over_changed_content_drops_what_is_gone() {
    let mut story = story();
    story.start_quest("varga_job").unwrap();
    story.run("journal('earth_mother')").unwrap();
    let mut save = story.save();
    save.facts.insert("retired.flag".into(), Value::Bool(true));
    save.quests
        .quests
        .insert("cut_quest".into(), Default::default());
    let (_, notes) = Story::load(Arc::clone(story.database()), &save).unwrap();
    assert_eq!(notes.len(), 2, "{notes:?}");
}

#[test]
fn a_quest_loop_is_stopped_and_reported() {
    let mut sources = content();
    sources.add_file(
        "quests/loop.somquest",
        r#"{"version": 1, "id": "loop", "title": "Loop", "stages": [
            {"id": "a", "next": [{"when": "true", "to": "b"}]},
            {"id": "b", "next": [{"when": "true", "to": "a"}]}]}"#,
    );
    let (database, _) = StoryDatabase::build(sources).unwrap();
    let mut story = Story::new(Arc::new(database), 1);
    story.start_quest("loop").unwrap();
    assert!(story.drain_events().iter().any(
        |e| matches!(e, StoryEvent::Problem { message } if message.contains("did not settle"))
    ));
}

#[test]
fn barks_respect_triggers_cooldowns_once_and_priority() {
    let mut story = story();
    let first = story.bark("spotted", &["watch_guard"]).unwrap();
    assert_eq!(first.speaker, "watch_guard");
    assert_ne!(first.id, "spotted_lidless", "its condition does not hold");
    assert!(
        story.bark("spotted", &["innkeeper"]).is_none(),
        "the unattributed bark needs lidless >= 3"
    );
    story.set("lidless", 3).unwrap();
    let urgent = story.bark("spotted", &["innkeeper"]).unwrap();
    assert_eq!(
        (urgent.id.as_str(), urgent.speaker.as_str()),
        ("spotted_lidless", "innkeeper")
    );
    let next = story.bark("spotted", &["watch_guard"]).unwrap();
    assert_ne!(next.id, "spotted_lidless", "once only");
    let mut counts = BTreeMap::new();
    for _ in 0..400 {
        story.tick(10_000);
        let pick = story.bark("spotted", &["watch_guard"]).unwrap();
        *counts.entry(pick.id).or_insert(0) += 1;
    }
    let (one, two) = (counts["spotted_1"], counts["spotted_2"]);
    assert!(two > one * 2, "weight 3 against 1: {counts:?}");
    assert!(story.bark("nothing_happens", &["watch_guard"]).is_none());
    let chatter = story.chatter("ambient", 2).unwrap();
    assert_eq!(chatter.roles, ["a", "b"]);
    assert_eq!(
        chatter.lines.iter().map(|l| l.role).collect::<Vec<_>>(),
        [0, 1, 0]
    );
    assert!(
        story.chatter("ambient", 1).is_none(),
        "two roles need two people"
    );
}

#[test]
fn notes_arrive_once_and_can_be_frosted() {
    let mut story = story();
    story.run("journal('earth_mother')").unwrap();
    story.run("journal('earth_mother')").unwrap();
    assert_eq!(story.journal().entries().len(), 1);
    story.run("journal_obscure('earth_mother')").unwrap();
    let view = story.journal_view();
    assert!(view.notes[0].obscured);
    assert_eq!(view.notes[0].title, "Ma");
    assert_eq!(view.unread, 1);
    story.mark_read(None);
    assert_eq!(story.journal_view().unread, 0);
    assert!(story.drain_events().contains(&StoryEvent::JournalObscured {
        note: "earth_mother".into(),
        obscured: true
    }));
}

#[test]
fn translations_resolve_with_the_source_as_fallback() {
    let mut story = story();
    let mut catalog = somnium_i18n::Catalog::new("en");
    catalog.insert(
        somnium_i18n::Table::new("de").with("dlg.mireille_intro.n1", "Du bist spät, kleiner Herr."),
    );
    catalog.set_locale("de");
    story.set_catalog(catalog);
    let mut talk = Conversation::start(&mut story, "mireille_intro", "").unwrap();
    assert_eq!(line(&talk).text, "Du bist spät, kleiner Herr.");
    talk.advance(&mut story).unwrap();
    talk.choose(&mut story, 0).unwrap();
    assert_eq!(
        line(&talk).text,
        "Of course you were.",
        "untranslated lines read as the source"
    );
}

#[test]
fn conditions_and_effects_run_from_text() {
    let mut story = story();
    assert!(
        story
            .check("not met.mireille and quest('varga_job') == 'inactive'")
            .unwrap()
    );
    assert!(story.check("met.mirielle").is_err());
    story
        .run("start_quest('varga_job'); give_item('lantern', 2)")
        .unwrap();
    assert!(story.check("stage('varga_job') == 'offer'").unwrap());
    assert!(story.drain_events().contains(&StoryEvent::Command {
        name: "give_item".into(),
        args: vec![Value::Text("lantern".into()), Value::Int(2)]
    }));
}
