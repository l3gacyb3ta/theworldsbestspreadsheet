//! Headless UI tests: drive the real app with synthetic input and render
//! snapshots to target/ui-shots/ for inspection.

use super::*;
use egui_kittest::{kittest::Queryable, Harness};

fn harness() -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size([1440.0, 900.0])
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
