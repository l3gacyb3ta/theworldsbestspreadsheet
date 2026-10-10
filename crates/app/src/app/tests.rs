//! Headless UI tests: drive the real app with synthetic input and render
//! snapshots to target/ui-shots/ for inspection.

use super::*;
use egui_kittest::{kittest::Queryable, Harness};

fn harness() -> Harness<'static, App> {
    harness_dt(0.25)
}

/// A harness whose frames are `dt` seconds apart (short enough for double clicks).
fn harness_dt(dt: f32) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1440.0, 900.0])
        .with_step_dt(dt)
        .wgpu()
        .build_eframe(|_| App::new(PathBuf::from("/nonexistent/ui-test.wbs.json")));
    h.run_steps(3);
    h
}

fn center(h: &Harness<'static, App>, at: &str) -> Pos2 {
    let r = a1::parse_ref(at).unwrap();
    h.state().geo.as_ref().unwrap().cell(r.row, r.col).center()
}

fn press(h: &mut Harness<'static, App>, pos: Pos2, pressed: bool, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: mods });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, App>, pos: Pos2, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    press(h, pos, true, mods);
    press(h, pos, false, mods);
    h.run_steps(2);
}

fn drag(h: &mut Harness<'static, App>, from: Pos2, to: Pos2, mods: Modifiers) {
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    press(h, from, true, mods);
    for i in 1..=6 {
        let p = from + (to - from) * (i as f32 / 6.0);
        h.event(Event::PointerMoved(p));
        h.run_steps(1);
    }
    press(h, to, false, mods);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

fn typ(h: &mut Harness<'static, App>, s: &str) {
    h.event(Event::Text(s.into()));
    h.run_steps(2);
}

fn key(h: &mut Harness<'static, App>, k: Key) {
    h.key_press(k);
    h.run_steps(2);
}

fn shown(h: &Harness<'static, App>, at: &str) -> String {
    let app = h.state();
    let r = a1::parse_ref(at).unwrap();
    let k = app.eng.wb.sheets[app.sheet_ix].key(r.row, r.col).unwrap();
    match app.eng.shown(k) {
        Shown::Empty => String::new(),
        Shown::Value { value, dr, dc, .. } => value.display_at(dr, dc),
        Shown::Error(e) => format!("ERR {}", e.msg),
    }
}

fn source(h: &Harness<'static, App>, at: &str) -> String {
    let app = h.state();
    let r = a1::parse_ref(at).unwrap();
    let k = app.eng.wb.sheets[app.sheet_ix].key(r.row, r.col).unwrap();
    app.eng.wb.cell_text(k)
}

fn shot(h: &mut Harness<'static, App>, name: &str) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-shots");
    std::fs::create_dir_all(&dir).unwrap();
    let img = h.render().expect("render");
    img.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
fn demo_renders() {
    let mut h = harness();
    shot(&mut h, "01_demo");
    assert_eq!(shown(&h, "G22"), "4");
}

#[test]
fn type_a_program() {
    let mut h = harness();
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=B3 2 *");
    shot(&mut h, "02_typing");
    key(&mut h, Key::Enter);
    assert_eq!(source(&h, "H25"), "=B3 2 *");
    assert_eq!(shown(&h, "H25"), "240,000 USD");
    // selection moved down
    assert_eq!(h.state().cursor, (25, 7));
}

#[test]
fn click_to_reference() {
    let mut h = harness();
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=");
    { let p = center(&h, "B3"); click(&mut h, p, Modifiers::NONE); }
    { let p = center(&h, "B4"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "*");
    shot(&mut h, "03_click_ref");
    key(&mut h, Key::Enter);
    assert_eq!(source(&h, "H25"), "=B3 B4 *");
    assert_eq!(shown(&h, "H25"), "4,800 USD");
}

#[test]
fn drag_range_reference() {
    let mut h = harness();
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=");
    { let p2 = center(&h, "A15"); let p = center(&h, "A10"); drag(&mut h, p, p2, Modifiers::NONE); }
    typ(&mut h, "sum");
    key(&mut h, Key::Enter);
    assert_eq!(source(&h, "H25"), "=A10:A15 sum");
    assert_eq!(shown(&h, "H25"), "21");
}

#[test]
fn fill_drag_series_and_formulas() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "1");
    key(&mut h, Key::Enter);
    typ(&mut h, "2");
    key(&mut h, Key::Enter);
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    { let p = center(&h, "E26"); click(&mut h, p, Modifiers::SHIFT); }
    let handle = h.state().geo.as_ref().unwrap().cell(25, 4).right_bottom() - Vec2::new(1.0, 1.0);
    let to = center(&h, "E30");
    drag(&mut h, handle, to, Modifiers::NONE);
    shot(&mut h, "04_fill");
    assert_eq!(source(&h, "E30"), "6");
    // formula fill
    { let p = center(&h, "F25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=E25 10 *");
    key(&mut h, Key::Enter);
    { let p = center(&h, "F25"); click(&mut h, p, Modifiers::NONE); }
    let handle = h.state().geo.as_ref().unwrap().cell(24, 5).right_bottom() - Vec2::new(1.0, 1.0);
    let to = center(&h, "F28");
    drag(&mut h, handle, to, Modifiers::NONE);
    assert_eq!(source(&h, "F28"), "=E28 10 *");
    assert_eq!(shown(&h, "F28"), "40");
    // undo removes the fill
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(source(&h, "F28"), "");
}

#[test]
fn scrub_input_resizes_spill() {
    let mut h = harness();
    assert_eq!(shown(&h, "A33"), "24");
    let from = center(&h, "B5");
    drag(&mut h, from, from - Vec2::new(40.0, 0.0), Modifiers::ALT);
    assert_eq!(source(&h, "B5"), "14");
    assert_eq!(shown(&h, "A23"), "14");
    assert_eq!(shown(&h, "A24"), "");
    shot(&mut h, "05_scrubbed");
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(2);
    assert_eq!(source(&h, "B5"), "24");
}

#[test]
fn drag_bar_writes_literal() {
    let mut h = harness();
    // scroll down to the bar chart
    let p = center(&h, "E20");
    h.hover_at(p);
    h.run_steps(1);
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -700.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
    h.run_steps(20);
    shot(&mut h, "06_bars");
    let hit = h
        .state()
        .chart_hits
        .iter()
        .find(|(p, _)| matches!(p.prov, Prov::Literal(_)) && p.label.starts_with("Q4"))
        .map(|(p, _)| p.pos)
        .expect("Q4 bar is draggable");
    drag(&mut h, hit, hit - Vec2::new(0.0, 40.0), Modifiers::NONE);
    shot(&mut h, "07_bar_dragged");
    let v: f64 = source(&h, "B47").split_whitespace().next().unwrap().parse().unwrap();
    assert!(v > 180.0, "{v}");
    assert!(source(&h, "B47").ends_with("[widget]"));
    // an integer literal stays an integer
    assert!(!source(&h, "B47").contains('.'), "{}", source(&h, "B47"));
    // a literal with decimals keeps them
    let k = h.state().eng.wb.sheets[0].key(45, 1).unwrap();
    h.state_mut().eng.set_text(k, "135.00 [widget]");
    h.run_steps(2);
    let hit = h.state().chart_hits.iter().find(|(p, _)| p.label.starts_with("Q3")).map(|(p, _)| p.pos).unwrap();
    drag(&mut h, hit, hit - Vec2::new(0.0, 23.0), Modifiers::NONE);
    let src = source(&h, "B46");
    assert!(src != "135.00 [widget]" && src.ends_with("[widget]"), "{src}");
    assert_eq!(src.split_whitespace().next().unwrap().split('.').nth(1).map(str::len), Some(2), "{src}");
}

#[test]
fn errors_point_at_the_token() {
    let mut h = harness();
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=B3 B5 +");
    key(&mut h, Key::Enter);
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    shot(&mut h, "08_error");
    assert!(shown(&h, "H25").contains("needs matching units"), "{}", shown(&h, "H25"));
}

#[test]
fn formula_extension_offer() {
    let mut h = harness();
    for (i, v) in ["1", "2", "3", "4"].iter().enumerate() {
        let p = center(&h, &format!("E{}", 25 + i));
        click(&mut h, p, Modifiers::NONE);
        typ(&mut h, v);
        key(&mut h, Key::Enter);
    }
    { let p = center(&h, "F25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=E25 dup *");
    key(&mut h, Key::Enter);
    assert_eq!(source(&h, "F25"), "=E25 dup *");
    assert_eq!(source(&h, "E28"), "4");
    assert!(h.state().offer.is_some(), "offer to extend");
    shot(&mut h, "09_offer");
    h.key_press_modifiers(Modifiers::COMMAND, Key::E);
    h.run_steps(2);
    assert_eq!(source(&h, "F28"), "=E28 dup *");
    assert_eq!(shown(&h, "F28"), "16");
    assert_eq!(source(&h, "F29"), "");
}

// ---- help system -------------------------------------------------------------

#[test]
fn f1_opens_help_for_the_selected_cell() {
    let mut h = harness();
    { let p = center(&h, "B10"); click(&mut h, p, Modifiers::NONE); }
    key(&mut h, Key::F1);
    assert!(h.state().help.open);
    assert_eq!(*h.state().help.page(), Page::Topic("stack"));
    h.run_steps(3);
    shot(&mut h, "10_help_stack");
    key(&mut h, Key::F1);
    assert!(!h.state().help.open, "F1 again closes help");
}

#[test]
fn search_shortcut_opens_help_and_f1_closes_it() {
    let mut h = harness();
    assert!(!h.state().help.open);
    h.key_press_modifiers(Modifiers::COMMAND, Key::Slash);
    h.run_steps(2);
    assert!(h.state().help.open);
    assert!(h.state().help.embedded(), "the headless harness embeds viewports");
    typ(&mut h, "dup");
    h.get_by_label("dup  Duplicates the top value.");
    key(&mut h, Key::F1);
    assert!(!h.state().help.open);
}

/// With multi-viewport on, help is its own viewport. The harness has no window
/// backend, so egui draws it embedded, but the app runs its separate-window logic.
fn harness_viewports() -> Harness<'static, App> {
    let mut h = harness();
    h.ctx.set_embed_viewports(false);
    h.run_steps(2);
    h
}

#[test]
fn help_window_is_its_own_viewport() {
    let mut h = harness_viewports();
    { let p = center(&h, "B10"); click(&mut h, p, Modifiers::NONE); }
    key(&mut h, Key::F1);
    assert!(h.state().help.open);
    assert!(!h.state().help.embedded());
    assert_eq!(*h.state().help.page(), Page::Topic("stack"));
    h.run_steps(3);
    h.get_by_label(help_view::TITLE);
    shot(&mut h, "18_help_viewport");
    // F1 in the main window navigates the open help window instead of closing it
    h.state_mut().select(2, 1, false);
    key(&mut h, Key::F1);
    assert!(h.state().help.open, "F1 from the main window keeps the help window open");
    assert_ne!(*h.state().help.page(), Page::Topic("stack"));
    // the toolbar button brings it forward rather than toggling it
    h.state_mut().help.show();
    h.run_steps(2);
    assert!(h.state().help.open);
    // closing it (its close button or F1 in it) and reopening
    h.state_mut().help.close();
    h.run_steps(2);
    assert!(!h.state().help.open && h.query_by_label(help_view::TITLE).is_none());
    key(&mut h, Key::F1);
    assert!(h.state().help.open);
    assert!(!h.state().help.has_focus(), "no OS window, so never focused");
}

#[test]
fn help_links_reach_the_main_window() {
    let mut h = harness_viewports();
    h.state_mut().help.show_page(Page::Units);
    h.run_steps(3);
    let (k, label) = {
        let app = h.state();
        let (_, k, _) = app.eng.units_list().into_iter().find(|(_, k, _)| app.eng.wb.pos(*k).is_some_and(|p| p != app.cursor)).unwrap();
        (k, app.eng.wb.cell_label(k, None))
    };
    h.query_all_by_label(&label).next().unwrap().click();
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.eng.wb.sheets[app.sheet_ix].id, k.sheet);
    assert_eq!(Some(app.cursor), app.eng.wb.pos(k));
}

#[test]
fn help_geometry_round_trips() {
    let mut help = Help::new();
    assert_eq!(help.geometry(), None);
    help.set_geometry("40 60 900 700");
    assert_eq!(help.geometry().as_deref(), Some("40 60 900 700"));
    let mut bad = Help::new();
    bad.set_geometry("nonsense");
    bad.set_geometry("0 0 10 10");
    assert_eq!(bad.geometry(), None);
}

#[test]
fn f1_while_editing_explains_the_word_at_the_cursor() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=A10:A15 sum");
    shot(&mut h, "11_assist_strip");
    key(&mut h, Key::F1);
    assert_eq!(*h.state().help.page(), Page::Word("sum"));
    h.run_steps(3);
    shot(&mut h, "12_help_word_sum");
}

#[test]
fn completions_insert_words() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=A10:A15 mea");
    h.run_steps(2);
    // the completion button is labelled with the word
    h.get_by_label("mean").click();
    h.run_steps(3);
    assert_eq!(h.state().edit.as_ref().unwrap().text, "=A10:A15 mean ");
    key(&mut h, Key::Enter);
    assert_eq!(shown(&h, "E25"), "3.5");
}

fn edit_text(h: &Harness<'static, App>) -> Option<String> {
    h.state().edit.as_ref().map(|e| e.text.clone())
}

#[test]
fn completions_from_the_keyboard() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=A10:A15 m");
    let cands = h.state().completions().unwrap().cands;
    assert!(cands.len() >= 2, "{cands:?}");
    // nothing is highlighted until ↓; ↓ ↓ ↑ ↓ lands on the second
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowUp);
    key(&mut h, Key::ArrowDown);
    assert_eq!(h.state().edit.as_ref().unwrap().pick, Some(1));
    shot(&mut h, "24_completion_highlighted");
    key(&mut h, Key::Enter);
    // inserted, still editing, caret after the word: typing carries on
    assert_eq!(edit_text(&h).unwrap(), format!("=A10:A15 {} ", cands[1]));
    assert_eq!(h.state().cursor, (24, 4));
    typ(&mut h, "x");
    assert_eq!(edit_text(&h).unwrap(), format!("=A10:A15 {} x", cands[1]));
    shot(&mut h, "25_completion_accepted");
    // ↑ above the first suggestion clears the highlight, so Enter commits as typed
    key(&mut h, Key::Escape);
    key(&mut h, Key::Escape);
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=A10:A15 mea");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowUp);
    assert_eq!(h.state().edit.as_ref().unwrap().pick, None);
    // Tab with the list showing but nothing highlighted keeps commit-and-move-right
    key(&mut h, Key::Tab);
    assert!(h.state().edit.is_none());
    assert_eq!(source(&h, "E25"), "=A10:A15 mea");
    assert_eq!(h.state().cursor, (24, 5));
}

#[test]
fn tab_accepts_and_enter_without_a_list_commits() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=A10:A15 mea");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::Tab);
    assert_eq!(edit_text(&h).unwrap(), "=A10:A15 mean ");
    // no partial word at the caret: no list, ↓/↑ do nothing harmful, Enter commits and moves down
    assert!(h.state().completions().is_none());
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::ArrowUp);
    assert_eq!(edit_text(&h).unwrap(), "=A10:A15 mean ");
    key(&mut h, Key::Enter);
    assert!(h.state().edit.is_none());
    assert_eq!(shown(&h, "E25"), "3.5");
    assert_eq!(h.state().cursor, (25, 4));
}

#[test]
fn escape_closes_completions_then_cancels() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    let top = grid_top(&h);
    typ(&mut h, "=A10:A15 mea");
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::Escape);
    assert_eq!(edit_text(&h).as_deref(), Some("=A10:A15 mea"), "first Escape only closes the list");
    assert!(h.state().completions().is_none());
    shot(&mut h, "26_completion_closed");
    // it stays closed until the text changes
    key(&mut h, Key::ArrowDown);
    assert!(h.state().completions().is_none());
    typ(&mut h, "n");
    assert!(h.state().completions().is_none(), "no completion for a whole word");
    typ(&mut h, " m");
    assert!(h.state().completions().is_some(), "typing reopens the list");
    key(&mut h, Key::Escape);
    key(&mut h, Key::Escape);
    assert!(h.state().edit.is_none(), "second Escape cancels the edit");
    assert_eq!(source(&h, "E25"), "");
    assert_eq!(grid_top(&h), top);
}

#[test]
fn unit_completion_from_the_keyboard() {
    let mut h = harness();
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    // the demo declares USD; complete it inside `[…`
    typ(&mut h, "=3 [U");
    let c = h.state().completions().expect("unit completions");
    assert!(c.unit);
    assert_eq!(c.cands[0], "USD");
    key(&mut h, Key::ArrowDown);
    shot(&mut h, "27_unit_completion");
    key(&mut h, Key::Enter);
    assert_eq!(edit_text(&h).unwrap(), "=3 [USD");
    typ(&mut h, "]");
    key(&mut h, Key::Enter);
    assert_eq!(shown(&h, "H25"), "3 USD");
}

#[test]
fn completions_work_in_the_formula_bar() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    // start in the cell, then click the formula bar (right of the address label): the bar editor gets focus
    typ(&mut h, "=A10:A15 mea");
    let addr = h.get_all_by_label("E25").map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap().right_center();
    click(&mut h, addr + Vec2::new(400.0, 0.0), Modifiers::NONE);
    assert!(h.state().edit.as_ref().unwrap().in_bar);
    key(&mut h, Key::End);
    key(&mut h, Key::ArrowDown);
    key(&mut h, Key::Enter);
    assert_eq!(edit_text(&h).unwrap(), "=A10:A15 mean ");
    key(&mut h, Key::Enter);
    assert_eq!(shown(&h, "E25"), "3.5");
}

#[test]
fn errors_are_explained_and_linked() {
    let mut h = harness();
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=B3 B5 +");
    key(&mut h, Key::Enter);
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    h.run_steps(2);
    h.get_by_label("Units don't match");
    shot(&mut h, "13_error_explained");
    key(&mut h, Key::F1);
    assert_eq!(*h.state().help.page(), Page::Topic("units"));
}

#[test]
fn dimensions_show_without_values() {
    let mut h = harness();
    // H40 is empty: no value, but the dimension is known statically
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=H40 drop 5 [m] 2 [s] /");
    h.run_steps(2);
    h.get_by_label("  dims: length/time");
    key(&mut h, Key::Enter);
    { let p = center(&h, "E25"); click(&mut h, p, Modifiers::NONE); }
    h.run_steps(2);
    assert!(shown(&h, "E25").contains("is empty"), "{}", shown(&h, "E25"));
    h.get_by_label("dimension: length/time");
    // a unit mismatch shows up while the input is still empty
    type_into(&mut h, "E26", "=H40 1 [m] 1 [s] + +");
    assert!(shown(&h, "E26").contains("+ needs matching units: length vs time"), "{}", shown(&h, "E26"));
}

#[test]
fn help_pages_render() {
    let mut h = harness();
    h.state_mut().help.show_page(Page::Reference);
    h.run_steps(3);
    shot(&mut h, "14_help_reference");
    h.state_mut().help.show_page(Page::Units);
    h.run_steps(3);
    shot(&mut h, "15_help_units");
    h.state_mut().help.try_in_playground("A1:A5 B1:B5 * sum", help_view::PlayCtx::Sample);
    h.run_steps(3);
    h.get_by_label("⇒ 550 m");
    shot(&mut h, "16_playground");
    h.state_mut().help.show_page(Page::YourWords);
    h.run_steps(3);
    h.get_by_label("npv");
    // every guide renders without panicking
    for t in help::topics() {
        h.state_mut().help.show_page(Page::Topic(t.id));
        h.run_steps(2);
    }
    h.state_mut().help.show_page(Page::Topic("units"));
    h.run_steps(3);
    shot(&mut h, "17_help_units_topic");
}

// ---- files: dirty tracking, New / Open / Save, the save-changes prompt ----

type Answers = std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<Option<PathBuf>>>>;

/// Scripted file dialogs: each call takes the next answer (None = cancelled).
struct Scripted(Answers);

impl files::Dialogs for Scripted {
    fn open(&mut self, _: Option<&std::path::Path>) -> Option<PathBuf> {
        self.0.borrow_mut().pop_front().expect("unexpected open dialog")
    }
    fn save_as(&mut self, _: Option<&std::path::Path>, _: &str) -> Option<PathBuf> {
        self.0.borrow_mut().pop_front().expect("unexpected save dialog")
    }
}

/// Install dialogs that answer `answers` in order; returns the queue so the
/// test can check they were all used.
fn script(h: &mut Harness<'static, App>, answers: &[Option<PathBuf>]) -> Answers {
    let q: Answers = std::rc::Rc::new(std::cell::RefCell::new(answers.iter().cloned().collect()));
    h.state_mut().dialogs = Box::new(Scripted(q.clone()));
    q
}

fn tmp_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wbs-ui-tests-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    let _ = std::fs::remove_file(&p);
    p
}

fn command(h: &mut Harness<'static, App>, c: Command) {
    h.state_mut().queue(c);
    h.run_steps(3);
}

fn key_cmd(h: &mut Harness<'static, App>, k: Key) {
    h.key_press_modifiers(Modifiers::COMMAND, k);
    h.run_steps(2);
}

/// Click a button in the save-changes prompt (drawn last, above the toolbar's Save).
fn modal_button(h: &mut Harness<'static, App>, label: &str) {
    h.get_all_by_label(label).last().expect("no such button").click();
}

fn type_into(h: &mut Harness<'static, App>, at: &str, text: &str) {
    let p = center(h, at);
    click(h, p, Modifiers::NONE);
    typ(h, text);
    key(h, Key::Enter);
}

#[test]
fn dirty_marker_follows_edits_and_undo() {
    let mut h = harness();
    assert!(!h.state().dirty);
    assert_eq!(h.state().title, "ui-test.wbs.json — the world's best spreadsheet");
    type_into(&mut h, "H25", "42");
    assert!(h.state().dirty);
    assert!(h.state().title.starts_with("• ui-test.wbs.json"));
    // undoing back to the saved state is clean again
    key_cmd(&mut h, Key::Z);
    assert_eq!(source(&h, "H25"), "");
    assert!(!h.state().dirty, "undo back to the saved state");
    // moving around grows the sheet but isn't an edit
    for _ in 0..40 {
        h.key_press(Key::ArrowDown);
    }
    h.run_steps(2);
    assert!(!h.state().is_dirty());
    // non-undoable changes count too: a column resize, a sheet rename
    let ix = h.state().sheet_ix;
    let cid = h.state().eng.wb.sheets[ix].cols.ids()[20];
    h.state_mut().eng.wb.sheets[ix].col_widths.insert(cid, 180.0);
    assert!(h.state().is_dirty());
    h.state_mut().eng.wb.sheets[ix].col_widths.remove(&cid);
    assert!(!h.state().is_dirty());
    h.state_mut().eng.wb.sheets[ix].name = "Renamed".into();
    assert!(h.state().is_dirty());
}

#[test]
fn save_as_then_save() {
    let mut h = harness();
    let p = tmp_file("save_as.wbs.json");
    let q = script(&mut h, &[None, Some(p.clone())]);
    type_into(&mut h, "H25", "42");
    // cancelled dialog: nothing written, still dirty
    command(&mut h, Command::SaveAs);
    assert!(!p.exists());
    assert!(h.state().dirty);
    command(&mut h, Command::SaveAs);
    assert!(p.exists());
    assert_eq!(h.state().path.as_deref(), Some(p.as_path()));
    assert!(!h.state().dirty);
    assert!(h.state().title.starts_with("save_as.wbs.json"));
    // ⌘S saves in place without asking
    type_into(&mut h, "H26", "43");
    assert!(h.state().dirty);
    key_cmd(&mut h, Key::S);
    assert!(!h.state().dirty);
    assert!(q.borrow().is_empty());
    let saved: wbs_core::model::Workbook = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    assert!(saved.sheets[0].cells.values().any(|c| c.pieces == vec![wbs_core::model::Piece::Text("43".into())]));
}

#[test]
fn new_asks_before_discarding_changes() {
    let mut h = harness();
    type_into(&mut h, "H25", "42");
    command(&mut h, Command::New);
    assert_eq!(h.state().confirm, Some(files::Pending::New));
    shot(&mut h, "18_save_changes_prompt");
    // Cancel keeps everything
    modal_button(&mut h, "Cancel");
    h.run_steps(3);
    assert_eq!(h.state().confirm, None);
    assert_eq!(source(&h, "H25"), "42");
    // Don't save: an empty workbook, untitled and clean
    command(&mut h, Command::New);
    modal_button(&mut h, "Don't save");
    h.run_steps(3);
    let app = h.state();
    assert_eq!(app.confirm, None);
    assert_eq!(app.path, None);
    assert!(!app.dirty);
    assert_eq!(app.title, "Untitled — the world's best spreadsheet");
    assert!(app.eng.wb.sheets[0].cells.is_empty());
    assert!(app.undo.is_empty());
    // a clean workbook doesn't ask
    command(&mut h, Command::New);
    assert_eq!(h.state().confirm, None);
}

#[test]
fn save_on_untitled_asks_where_and_open_reads_it_back() {
    let mut h = harness();
    command(&mut h, Command::New);
    let p = tmp_file("roundtrip.wbs.json");
    let q = script(&mut h, &[Some(p.clone()), Some(p.clone())]);
    type_into(&mut h, "A1", "=6 7 *");
    key_cmd(&mut h, Key::S);
    assert_eq!(h.state().path.as_deref(), Some(p.as_path()));
    command(&mut h, Command::New);
    assert_eq!(source(&h, "A1"), "");
    command(&mut h, Command::Open);
    assert!(q.borrow().is_empty());
    assert_eq!(h.state().path.as_deref(), Some(p.as_path()));
    assert_eq!(source(&h, "A1"), "=6 7 *");
    assert_eq!(shown(&h, "A1"), "42");
    assert!(!h.state().dirty);
}

#[test]
fn open_with_changes_can_save_first() {
    let mut h = harness();
    let mine = tmp_file("mine.wbs.json");
    let other = tmp_file("other.wbs.json");
    std::fs::write(&other, serde_json::to_string(&wbs_core::stdlib::default_workbook()).unwrap()).unwrap();
    h.state_mut().path = Some(mine.clone());
    let q = script(&mut h, &[Some(other.clone())]);
    type_into(&mut h, "H25", "42");
    command(&mut h, Command::Open);
    assert_eq!(h.state().confirm, Some(files::Pending::Open));
    modal_button(&mut h, "Save");
    h.run_steps(3);
    assert!(mine.exists(), "saved before opening");
    assert!(q.borrow().is_empty());
    assert_eq!(h.state().path.as_deref(), Some(other.as_path()));
    // a file that isn't a workbook leaves the current one alone
    let bad = tmp_file("bad.wbs.json");
    std::fs::write(&bad, "not json").unwrap();
    script(&mut h, &[Some(bad)]);
    command(&mut h, Command::Open);
    assert_eq!(h.state().path.as_deref(), Some(other.as_path()));
    assert!(h.state().status.as_deref().unwrap().starts_with("couldn't open"));
}

#[test]
fn closing_with_changes_asks() {
    let mut h = harness();
    let close = |h: &mut Harness<'static, App>| {
        h.input_mut().viewports.get_mut(&egui::ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
        h.run_steps(3);
    };
    // clean: closes without asking
    close(&mut h);
    assert_eq!(h.state().confirm, None);
    type_into(&mut h, "H25", "42");
    close(&mut h);
    assert_eq!(h.state().confirm, Some(files::Pending::Quit));
    assert!(!h.state().close_ok);
    modal_button(&mut h, "Cancel");
    h.run_steps(3);
    assert!(!h.state().close_ok);
    // Save to an unwritable path fails: the window stays open
    close(&mut h);
    modal_button(&mut h, "Save");
    h.run_steps(3);
    assert!(!h.state().close_ok);
    assert!(h.state().status.as_deref().unwrap().starts_with("save failed"));
    let p = tmp_file("quit.wbs.json");
    h.state_mut().path = Some(p.clone());
    close(&mut h);
    modal_button(&mut h, "Save");
    h.run_steps(3);
    assert!(p.exists());
    assert!(h.state().close_ok);
}

#[test]
fn edit_commands_reach_the_grid_and_the_editor() {
    let mut h = harness();
    type_into(&mut h, "H25", "42");
    command(&mut h, Command::Undo);
    assert_eq!(source(&h, "H25"), "");
    command(&mut h, Command::Redo);
    assert_eq!(source(&h, "H25"), "42");
    let trace = h.state().trace;
    command(&mut h, Command::ToggleTrace);
    assert_eq!(h.state().trace, !trace);
    // while typing, Undo belongs to the text field: it's replayed as ⌘Z
    // (fed back in by raw_input_hook in the real app)
    {
        let p = center(&h, "H26");
        click(&mut h, p, Modifiers::NONE);
    }
    typ(&mut h, "abc");
    h.state_mut().queue(Command::Undo);
    h.run_steps(1);
    let injected = std::mem::take(&mut h.state_mut().inject);
    assert!(matches!(injected[..], [Event::Key { key: Key::Z, pressed: true, modifiers, .. }] if modifiers.command));
    assert_eq!(source(&h, "H25"), "42", "grid undo didn't run");
}

// ---- sheet tabs ----------------------------------------------------------------

/// Short steps so two clicks count as a double-click.
fn quick_harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1440.0, 900.0])
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|_| App::new(PathBuf::from("/nonexistent/ui-test.wbs.json")));
    h.run_steps(3);
    h
}

fn sheet_names(h: &Harness<'static, App>) -> Vec<String> {
    h.state().eng.wb.sheets.iter().map(|s| s.name.clone()).collect()
}

fn tab(h: &Harness<'static, App>, name: &str) -> Pos2 {
    h.get_by_label(name).rect().center()
}

fn double_click(h: &mut Harness<'static, App>, pos: Pos2) {
    h.event(Event::PointerMoved(pos));
    h.run_steps(1);
    for _ in 0..2 {
        press(h, pos, true, Modifiers::NONE);
        press(h, pos, false, Modifiers::NONE);
    }
    h.run_steps(2);
}

fn tab_menu(h: &mut Harness<'static, App>, name: &str, item: &str) {
    h.get_by_label(name).click_secondary();
    h.run_steps(2);
    h.get_by_label(item).click();
    h.run_steps(3);
}

fn undo(h: &mut Harness<'static, App>) {
    h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
    h.run_steps(3);
}

#[test]
fn sheet_tabs_add_rename_delete_undo() {
    let mut h = quick_harness();
    assert_eq!(sheet_names(&h), ["model", "units"]);
    h.get_by_label("+").click();
    h.run_steps(30); // long enough that the next click isn't a double-click
    assert_eq!(sheet_names(&h), ["model", "units", "Sheet3"]);
    assert_eq!(h.state().sheet_ix, 2, "the new sheet is shown");
    // double-click renames inline
    { let p = tab(&h, "Sheet3"); double_click(&mut h, p); }
    assert!(h.state().tabs.rename.is_some());
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    typ(&mut h, "inputs");
    shot(&mut h, "18_sheet_renaming");
    key(&mut h, Key::Enter);
    assert_eq!(sheet_names(&h), ["model", "units", "inputs"]);
    assert!(h.state().tabs.rename.is_none());
    { let p = center(&h, "A1"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "7");
    key(&mut h, Key::Enter);
    // reference it from the model sheet
    h.get_by_label("model").click();
    h.run_steps(3);
    assert_eq!(h.state().sheet_ix, 0);
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=inputs!A1 2 *");
    key(&mut h, Key::Enter);
    assert_eq!(shown(&h, "H25"), "14");
    shot(&mut h, "19_sheet_tabs");
    // delete it from the tab's menu: the reference breaks visibly
    tab_menu(&mut h, "inputs", "Delete");
    assert_eq!(sheet_names(&h), ["model", "units"]);
    assert_eq!(h.state().sheet_ix, 0);
    assert_eq!(source(&h, "H25"), "=#ref! 2 *");
    assert_eq!(shown(&h, "H25"), "ERR reference to a deleted sheet");
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    shot(&mut h, "20_sheet_deleted");
    // undo brings the sheet and the reference back
    undo(&mut h);
    assert_eq!(sheet_names(&h), ["model", "units", "inputs"]);
    assert_eq!(h.state().sheet_ix, 2, "the restored sheet is shown");
    assert_eq!(shown(&h, "A1"), "7");
    h.get_by_label("model").click();
    h.run_steps(3);
    assert_eq!(source(&h, "H25"), "=inputs!A1 2 *");
    assert_eq!(shown(&h, "H25"), "14");
    // undo the formula, the 7, the rename, then the add; the shown sheet stays valid
    undo(&mut h);
    undo(&mut h);
    undo(&mut h);
    assert_eq!(sheet_names(&h), ["model", "units", "Sheet3"]);
    undo(&mut h);
    assert_eq!(sheet_names(&h), ["model", "units"]);
    assert!(h.state().sheet_ix < 2);
}

#[test]
fn sheet_rename_escape_and_invalid_names() {
    let mut h = quick_harness();
    { let p = tab(&h, "model"); double_click(&mut h, p); }
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    typ(&mut h, "units");
    key(&mut h, Key::Enter);
    assert_eq!(sheet_names(&h), ["model", "units"], "names stay unique");
    assert!(h.state().status.as_deref().unwrap_or("").contains("already a sheet"));
    { let p = tab(&h, "model"); double_click(&mut h, p); }
    typ(&mut h, "zzz");
    key(&mut h, Key::Escape);
    assert_eq!(sheet_names(&h), ["model", "units"], "escape cancels");
    assert!(h.state().undo.is_empty(), "nothing to undo");
}

#[test]
fn sheet_duplicate_move_and_drag() {
    let mut h = quick_harness();
    tab_menu(&mut h, "model", "Duplicate");
    assert_eq!(sheet_names(&h), ["model", "model copy", "units"]);
    assert_eq!(h.state().sheet_ix, 1);
    assert_eq!(shown(&h, "G22"), "4", "the copy computes on its own");
    tab_menu(&mut h, "model copy", "Move right");
    assert_eq!(sheet_names(&h), ["model", "units", "model copy"]);
    assert_eq!(h.state().sheet_ix, 2, "the moved sheet stays shown");
    // drag the last tab to the front
    let from = tab(&h, "model copy");
    let to = tab(&h, "model") - Vec2::new(20.0, 0.0);
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    press(&mut h, from, true, Modifiers::NONE);
    for i in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.run_steps(1);
    }
    assert_eq!(sheet_names(&h), ["model", "units", "model copy"], "nothing moves until the drop");
    shot(&mut h, "21_sheet_dragging");
    press(&mut h, to, false, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(sheet_names(&h), ["model copy", "model", "units"]);
    assert_eq!(h.state().sheet_ix, 0);
    shot(&mut h, "22_sheet_dragged");
    undo(&mut h);
    undo(&mut h);
    undo(&mut h);
    assert_eq!(sheet_names(&h), ["model", "units"]);
    assert!(h.state().sheet_ix < 2);
}

#[test]
fn deleting_units_asks_first() {
    let mut h = quick_harness();
    tab_menu(&mut h, "units", "Delete");
    assert_eq!(sheet_names(&h), ["model", "units"], "not yet");
    assert!(h.state().tabs.confirm_delete.is_some());
    shot(&mut h, "23_delete_units_confirm");
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().tabs.confirm_delete.is_none());
    tab_menu(&mut h, "units", "Delete");
    h.get_by_label("Delete sheet").click();
    h.run_steps(3);
    assert_eq!(sheet_names(&h), ["model"]);
    assert!(shown(&h, "B3").starts_with("ERR"), "{}", shown(&h, "B3"));
    shot(&mut h, "24_units_deleted");
    undo(&mut h);
    assert_eq!(sheet_names(&h), ["model", "units"]);
    h.get_by_label("model").click();
    h.run_steps(3);
    assert_eq!(shown(&h, "B3"), "120,000 USD");
}

// ---- papercuts -----------------------------------------------------------------

fn grid_top(h: &Harness<'static, App>) -> f32 {
    h.state().geo.as_ref().unwrap().cells.top()
}

#[test]
fn editing_keeps_the_grid_still() {
    let mut h = harness();
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    let top = grid_top(&h);
    shot(&mut h, "18_layout_idle");
    // typing a program brings up the hint and stack rows
    typ(&mut h, "=A10:A15 mea");
    assert_eq!(grid_top(&h), top);
    shot(&mut h, "19_layout_editing");
    key(&mut h, Key::Escape);
    assert!(h.state().edit.is_none());
    assert_eq!(grid_top(&h), top);
    // F2 on a number, and on text
    { let p = center(&h, "B3"); click(&mut h, p, Modifiers::NONE); }
    key(&mut h, Key::F2);
    assert!(h.state().edit.is_some());
    assert_eq!(grid_top(&h), top);
    key(&mut h, Key::Escape);
    { let p = center(&h, "A3"); click(&mut h, p, Modifiers::NONE); }
    key(&mut h, Key::F2);
    assert_eq!(grid_top(&h), top);
}

#[test]
fn double_click_edits_an_error_cell() {
    // 60 fps, so two clicks land inside egui's double-click window
    let mut h = harness_dt(1.0 / 60.0);
    { let p = center(&h, "H25"); click(&mut h, p, Modifiers::NONE); }
    typ(&mut h, "=B3 B5 +");
    key(&mut h, Key::Enter);
    h.run_steps(60);
    let p = center(&h, "H25");
    click(&mut h, p, Modifiers::NONE);
    assert!(h.state().edit.is_none());
    h.run_steps(60);
    shot(&mut h, "20_error_selected");
    // a real double click: press and release on separate frames; egui reports it on the second release
    click(&mut h, p, Modifiers::NONE);
    assert!(h.state().edit.is_none(), "one click only selects");
    click(&mut h, p, Modifiers::NONE);
    // the in-cell editor holds the source (with the bad `+` underlined), not "#err"
    assert_eq!(h.state().edit.as_ref().map(|e| e.text.as_str()), Some("=B3 B5 +"));
    assert_eq!(h.state().edit.as_ref().unwrap().key, h.state().eng.wb.sheets[0].key(24, 7).unwrap());
    h.event(Event::PointerMoved(p + Vec2::new(0.0, 80.0)));
    h.run_steps(2);
    shot(&mut h, "21_error_cell_editing");
    key(&mut h, Key::End);
    key(&mut h, Key::Backspace);
    typ(&mut h, "*");
    key(&mut h, Key::Enter);
    assert_eq!(source(&h, "H25"), "=B3 B5 *");
    assert_eq!(shown(&h, "H25"), "2,880,000 USD");
}

#[test]
fn wide_numbers_never_look_like_other_numbers() {
    let mut h = harness();
    let p = center(&h, "E20");
    h.hover_at(p);
    h.run_steps(1);
    h.event(Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -500.0), modifiers: Modifiers::NONE, phase: egui::TouchPhase::Move });
    h.run_steps(20);
    // H38 has an empty G38 to its left, so it runs into it; so does H40, next to a label that stops at F40;
    // H41's neighbours are taken by a longer label, so it shows ###
    let wide = "=123456 [km] to[mm]";
    for (at, text) in [("H38", wide), ("E40", "'a label that runs on"), ("H40", wide), ("E41", "'a label that runs on and on, right up to column H"), ("H41", wide)] {
        let p = center(&h, at);
        click(&mut h, p, Modifiers::NONE);
        typ(&mut h, text);
        key(&mut h, Key::Enter);
    }
    assert_eq!(shown(&h, "H38"), "123,456,000,000 mm");
    h.hover_at(center(&h, "C45"));
    h.run_steps(2);
    shot(&mut h, "22_wide_numbers");
    // hovering ### shows the value
    h.hover_at(center(&h, "H41"));
    h.run_steps(2);
    shot(&mut h, "23_wide_number_hover");
}

// ---- moving cells --------------------------------------------------------------

fn paste_clip(h: &mut Harness<'static, App>) {
    let text = h.state().clip.as_ref().expect("nothing copied").text.clone();
    h.event(Event::Paste(text));
    h.run_steps(2);
}

#[test]
fn cut_then_paste_moves_cells() {
    let mut h = harness();
    type_into(&mut h, "E24", "5");
    type_into(&mut h, "E25", "=E24 2 *");
    type_into(&mut h, "F24", "=E24 1 +");
    { let p = center(&h, "E24"); click(&mut h, p, Modifiers::NONE); }
    h.event(Event::Cut);
    h.run_steps(2);
    // a cut only marks the cells; nothing changes until the paste
    assert_eq!(source(&h, "E24"), "5");
    shot(&mut h, "24_cut_marked");
    { let p = center(&h, "G27"); click(&mut h, p, Modifiers::NONE); }
    paste_clip(&mut h);
    shot(&mut h, "25_cut_pasted");
    assert_eq!(source(&h, "E24"), "");
    assert_eq!(source(&h, "G27"), "5");
    assert_eq!(source(&h, "E25"), "=G27 2 *");
    assert_eq!(source(&h, "F24"), "=G27 1 +");
    assert_eq!(shown(&h, "E25"), "10");
    // one undo step puts everything back, and redo moves it again
    undo(&mut h);
    assert_eq!(source(&h, "E24"), "5");
    assert_eq!(source(&h, "G27"), "");
    assert_eq!(source(&h, "E25"), "=E24 2 *");
    command(&mut h, Command::Redo);
    assert_eq!(source(&h, "E25"), "=G27 2 *");
    // the cut is used up: pasting again copies the moved cells
    { let p = center(&h, "G28"); click(&mut h, p, Modifiers::NONE); }
    paste_clip(&mut h);
    assert_eq!(source(&h, "G28"), "5");
    assert_eq!(source(&h, "G27"), "5");
    assert_eq!(source(&h, "E25"), "=G27 2 *");
    // a cut interrupted by an edit doesn't move: the paste copies
    { let p = center(&h, "G27"); click(&mut h, p, Modifiers::NONE); }
    h.event(Event::Cut);
    h.run_steps(2);
    type_into(&mut h, "F29", "1");
    { let p = center(&h, "H27"); click(&mut h, p, Modifiers::NONE); }
    paste_clip(&mut h);
    assert_eq!(source(&h, "G27"), "5");
    assert_eq!(source(&h, "H27"), "5");
}

#[test]
fn spilled_cells_cant_be_cut_alone() {
    let mut h = harness();
    // A12 is part of the demo's spilled sequence
    assert_eq!(source(&h, "A12"), "");
    assert_eq!(shown(&h, "A12"), "3");
    { let p = center(&h, "A12"); click(&mut h, p, Modifiers::NONE); }
    h.event(Event::Cut);
    h.run_steps(2);
    { let p = center(&h, "G27"); click(&mut h, p, Modifiers::NONE); }
    paste_clip(&mut h);
    assert!(h.state().status.as_deref().unwrap_or("").contains("A12 is spilled"), "{:?}", h.state().status);
    assert_eq!(source(&h, "G27"), "");
}

#[test]
fn dragging_the_selection_border_moves_and_alt_copies() {
    let mut h = harness();
    type_into(&mut h, "E24", "5");
    type_into(&mut h, "F24", "=E24 2 *");
    { let p = center(&h, "E24"); click(&mut h, p, Modifiers::NONE); }
    // press just inside the selection's left edge and drag to G27, with a snapshot mid-drag
    let from = h.state().geo.as_ref().unwrap().cell(23, 4).left_center() + Vec2::new(1.0, 0.0);
    let to = h.state().geo.as_ref().unwrap().cell(26, 6).left_center() + Vec2::new(1.0, 0.0);
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    press(&mut h, from, true, Modifiers::NONE);
    for i in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.run_steps(1);
    }
    assert!(matches!(h.state().drag, Drag::Move { copy: false, .. }));
    shot(&mut h, "26_border_drag");
    press(&mut h, to, false, Modifiers::NONE);
    h.run_steps(2);
    assert_eq!(source(&h, "E24"), "");
    assert_eq!(source(&h, "G27"), "5");
    assert_eq!(source(&h, "F24"), "=G27 2 *");
    assert_eq!(h.state().cursor, (26, 6));
    shot(&mut h, "27_border_dropped");
    undo(&mut h);
    assert_eq!(source(&h, "E24"), "5");
    assert_eq!(source(&h, "F24"), "=E24 2 *");
    command(&mut h, Command::Redo);
    assert_eq!(source(&h, "G27"), "5");
    // Alt-drag on the border copies
    let from = h.state().geo.as_ref().unwrap().cell(26, 6).left_center() + Vec2::new(1.0, 0.0);
    let to = h.state().geo.as_ref().unwrap().cell(26, 7).left_center() + Vec2::new(1.0, 0.0);
    drag(&mut h, from, to, Modifiers::ALT);
    assert_eq!(source(&h, "G27"), "5");
    assert_eq!(source(&h, "H27"), "5");
    assert_eq!(source(&h, "F24"), "=G27 2 *");
    // Alt-drag inside the cell still scrubs
    let c = center(&h, "H27");
    drag(&mut h, c, c - Vec2::new(40.0, 0.0), Modifiers::ALT);
    assert_eq!(source(&h, "H27"), "-5");
    // a click just outside the selection's border selects the cell there
    let edge = h.state().geo.as_ref().unwrap().cell(26, 7).left_center() - Vec2::new(2.0, 0.0);
    click(&mut h, edge, Modifiers::NONE);
    assert_eq!(h.state().cursor, (26, 6));
    assert_eq!(source(&h, "G27"), "5");
}
