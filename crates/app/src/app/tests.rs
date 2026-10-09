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
