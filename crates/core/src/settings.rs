//! Settings: every user-configurable setting is declared once, in `SETTINGS`.
//! The settings window, storage, validation and the help page are all driven
//! from these declarations, so adding a setting is adding an entry here and
//! reading it where it's used.
//!
//! Two scopes: app preferences (per user and machine; the app keeps them in a
//! TOML file in the platform config dir) and workbook settings (in the
//! workbook file, `Workbook::settings`). This module does no I/O: it checks
//! stored values (as JSON values, whatever the file format) against the
//! declarations. An invalid stored value is reported and the default used; it
//! is never rewritten or dropped. Unknown keys are kept as they are.

use crate::model::Workbook;
use serde_json::Value as Json;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Per user and machine, in the app's settings file.
    App,
    /// Saved in the workbook file.
    Workbook,
}

impl Scope {
    pub fn title(self) -> &'static str {
        match self {
            Scope::App => "App preferences",
            Scope::Workbook => "This workbook",
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Kind {
    Bool,
    /// A whole number from `min` to `max` (inclusive), in `unit`.
    Int { min: i64, max: i64, unit: &'static str },
    /// Text, checked by `check` (which says why it's not acceptable).
    Text { check: fn(&str) -> Result<(), String> },
    /// One of these (stored value, label).
    Choice(&'static [(&'static str, &'static str)]),
    /// `#rrggbb`.
    Colour,
}

/// A setting's value. Choices are `Text` holding the stored value.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Text(String),
    Colour([u8; 3]),
}

/// A default, as a constant.
#[derive(Copy, Clone, Debug)]
pub enum Def {
    Bool(bool),
    Int(i64),
    Text(&'static str),
    Colour([u8; 3]),
}

#[derive(Debug)]
pub struct Setting {
    /// `section.name`; in the TOML file `[section]` then `name = …`.
    pub key: &'static str,
    pub scope: Scope,
    pub section: &'static str,
    pub label: &'static str,
    /// Mini-markdown, shown under the setting and on the help page.
    pub help: &'static str,
    pub kind: Kind,
    pub default: Def,
    /// Declared and stored, but nothing reads it yet: says what will.
    pub pending: Option<&'static str>,
}

const COLLAB: Option<&str> = Some("Not used yet: collaboration is coming in #18.");

pub const AUTOSAVE_ENABLED: &Setting = &Setting {
    key: "autosave.enabled",
    scope: Scope::App,
    section: "Autosave",
    label: "Autosave",
    help: "When on, a workbook that already has a file is saved to that file a set time after its first unsaved change. \
           An untitled workbook is never autosaved (nothing asks where to save it), and quitting still asks about changes made since. \
           The status bar shows when autosave is on and when it last saved.",
    kind: Kind::Bool,
    default: Def::Bool(true),
    pending: None,
};

pub const AUTOSAVE_INTERVAL: &Setting = &Setting {
    key: "autosave.interval_seconds",
    scope: Scope::App,
    section: "Autosave",
    label: "Autosave after",
    help: "How long after the first unsaved change autosave writes the file. It waits while you're typing in a cell or dragging.",
    kind: Kind::Int { min: 5, max: 3600, unit: "seconds" },
    default: Def::Int(60),
    pending: None,
};

pub const AUTOSAVE_WORKBOOK: &Setting = &Setting {
    key: "autosave.this_workbook",
    scope: Scope::Workbook,
    section: "Autosave",
    label: "Autosave this workbook",
    help: "Overrides the app's **Autosave** for this workbook only, for example to never autosave a template. Saved in the workbook file.",
    kind: Kind::Choice(&[("app", "as the app setting says"), ("always", "always"), ("never", "never")]),
    default: Def::Text("app"),
    pending: None,
};

pub const TRACE_AT_START: &Setting = &Setting {
    key: "view.trace",
    scope: Scope::App,
    section: "View",
    label: "Trace on at start",
    help: "Whether **Trace** (highlighting the selected cell's precedents and dependents) is on when the app starts. \
           Changing it here also switches Trace now; the toolbar button switches it for this session only.",
    kind: Kind::Bool,
    default: Def::Bool(true),
    pending: None,
};

pub const GOAL_SEEK_LIVE_MS: &Setting = &Setting {
    key: "charts.live_goal_seek_ms",
    scope: Scope::App,
    section: "Charts",
    label: "Live goal-seek limit",
    help: "Dragging a computed chart point goal-seeks an input on every frame. If one solve takes longer than this, \
           the drag stops following the pointer and solves once when you let go. Raise it on a fast machine, lower it if dragging stutters.",
    kind: Kind::Int { min: 1, max: 5000, unit: "ms" },
    default: Def::Int(50),
    pending: None,
};

pub const DISPLAY_NAME: &Setting = &Setting {
    key: "identity.display_name",
    scope: Scope::App,
    section: "Collaboration",
    label: "Your name",
    help: "The name other people see next to your selection when you share a workbook. It's self-declared: there are no accounts, \
           so anyone can type any name. Empty means you show as \"Anonymous\".",
    kind: Kind::Text { check: check_display_name },
    default: Def::Text(""),
    pending: COLLAB,
};

pub const COLOUR: &Setting = &Setting {
    key: "identity.colour",
    scope: Scope::App,
    section: "Collaboration",
    label: "Your colour",
    help: "The colour of your selection outline and name tag in other people's windows when you share a workbook.",
    kind: Kind::Colour,
    default: Def::Colour([0x0e, 0xa5, 0x72]),
    pending: COLLAB,
};

pub const SYNC_SERVER: &Setting = &Setting {
    key: "sync.server_url",
    scope: Scope::App,
    section: "Collaboration",
    label: "Sync server",
    help: "The server shared workbooks sync through: a `ws://` or `wss://` address. The default is the public Automerge server, \
           which needs no setup. **Its operator can read every workbook shared through it**: the server stores documents unencrypted. \
           For anything private, run your own server and put its address here.",
    kind: Kind::Text { check: check_sync_url },
    default: Def::Text("wss://sync.automerge.org"),
    pending: COLLAB,
};

/// Every setting, in the order the settings window and help page show them.
pub const SETTINGS: &[&Setting] = &[AUTOSAVE_ENABLED, AUTOSAVE_INTERVAL, AUTOSAVE_WORKBOOK, TRACE_AT_START, GOAL_SEEK_LIVE_MS, DISPLAY_NAME, COLOUR, SYNC_SERVER];

pub fn setting(key: &str) -> Option<&'static Setting> {
    SETTINGS.iter().copied().find(|s| s.key == key)
}

/// The sections that have settings in `scope`, in declaration order.
pub fn sections(scope: Scope) -> Vec<&'static str> {
    let mut out: Vec<&str> = Vec::new();
    for s in SETTINGS.iter().filter(|s| s.scope == scope) {
        if !out.contains(&s.section) {
            out.push(s.section);
        }
    }
    out
}

/// The keys among `keys` that no setting of `scope` declares (kept, but unused).
pub fn unknown_keys<'a>(scope: Scope, keys: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    keys.into_iter().filter(|k| !setting(k).is_some_and(|s| s.scope == scope)).map(str::to_string).collect()
}

fn check_display_name(s: &str) -> Result<(), String> {
    if s.contains(['\n', '\r', '\t']) {
        Err("must be on one line".into())
    } else if s.chars().count() > 40 {
        Err(format!("must be at most 40 characters (this is {})", s.chars().count()))
    } else {
        Ok(())
    }
}

fn check_sync_url(s: &str) -> Result<(), String> {
    let Some(rest) = s.strip_prefix("wss://").or_else(|| s.strip_prefix("ws://")) else {
        return Err("must start with wss:// or ws://".into());
    };
    let host_port = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host_port.is_empty() {
        return Err("needs a server name after the ://".into());
    }
    if s.chars().any(char::is_whitespace) {
        return Err("can't contain spaces".into());
    }
    if let Some((_, port)) = host_port.rsplit_once(':').filter(|_| !host_port.ends_with(']')) {
        if port.parse::<u16>().is_err() {
            return Err(format!("{port:?} isn't a port number"));
        }
    }
    Ok(())
}

pub fn parse_colour(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap();
    Some([b(0), b(2), b(4)])
}

pub fn colour_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// How a stored value reads in an error message.
fn describe(raw: &Json) -> String {
    match raw {
        Json::String(s) => format!("{s:?}"),
        Json::Number(n) => n.to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Array(_) => "a list".into(),
        Json::Object(_) => "a table".into(),
        Json::Null => "nothing".into(),
    }
}

impl Val {
    pub fn as_bool(&self) -> bool {
        matches!(self, Val::Bool(true))
    }
    pub fn as_int(&self) -> i64 {
        match self {
            Val::Int(n) => *n,
            _ => 0,
        }
    }
    pub fn as_text(&self) -> &str {
        match self {
            Val::Text(s) => s,
            _ => "",
        }
    }
    pub fn as_colour(&self) -> [u8; 3] {
        match self {
            Val::Colour(c) => *c,
            _ => [0, 0, 0],
        }
    }
}

impl Def {
    pub fn val(self) -> Val {
        match self {
            Def::Bool(b) => Val::Bool(b),
            Def::Int(n) => Val::Int(n),
            Def::Text(s) => Val::Text(s.to_string()),
            Def::Colour(c) => Val::Colour(c),
        }
    }
}

impl Setting {
    pub fn default_val(&self) -> Val {
        self.default.val()
    }

    /// Reads a stored value, or says why it isn't acceptable.
    pub fn parse(&self, raw: &Json) -> Result<Val, String> {
        let found = || describe(raw);
        match self.kind {
            Kind::Bool => raw.as_bool().map(Val::Bool).ok_or_else(|| format!("expected true or false, found {}", found())),
            Kind::Int { min, max, unit } => match raw.as_i64() {
                Some(n) if (min..=max).contains(&n) => Ok(Val::Int(n)),
                Some(n) => Err(format!("must be from {min} to {max} {unit}, found {n}")),
                None => Err(format!("expected a whole number of {unit}, found {}", found())),
            },
            Kind::Text { check } => match raw.as_str() {
                Some(s) => check(s).map(|_| Val::Text(s.to_string())),
                None => Err(format!("expected text in quotes, found {}", found())),
            },
            Kind::Choice(opts) => match raw.as_str() {
                Some(s) if opts.iter().any(|(v, _)| *v == s) => Ok(Val::Text(s.to_string())),
                _ => {
                    let list: Vec<String> = opts.iter().map(|(v, _)| format!("{v:?}")).collect();
                    Err(format!("expected one of {}, found {}", list.join(", "), found()))
                }
            },
            Kind::Colour => raw.as_str().and_then(parse_colour).map(Val::Colour).ok_or_else(|| format!("expected a colour like \"#0ea572\", found {}", found())),
        }
    }

    /// How a value is stored.
    pub fn to_json(&self, v: &Val) -> Json {
        match v {
            Val::Bool(b) => Json::Bool(*b),
            Val::Int(n) => Json::from(*n),
            Val::Text(s) => Json::String(s.clone()),
            Val::Colour(c) => Json::String(colour_hex(*c)),
        }
    }

    /// Whether `v` is acceptable (the same check as reading it back).
    pub fn check(&self, v: &Val) -> Result<(), String> {
        self.parse(&self.to_json(v)).map(|_| ())
    }

    /// The value to use for what's stored (`None`: not set), and why a stored value was rejected.
    pub fn resolve(&self, raw: Option<&Json>) -> (Val, Option<String>) {
        match raw.map(|r| self.parse(r)) {
            None => (self.default_val(), None),
            Some(Ok(v)) => (v, None),
            Some(Err(why)) => (self.default_val(), Some(why)),
        }
    }

    /// A value as the settings window and help page show it.
    pub fn show(&self, v: &Val) -> String {
        match (self.kind, v) {
            (Kind::Bool, Val::Bool(b)) => if *b { "on" } else { "off" }.into(),
            (Kind::Int { unit, .. }, Val::Int(n)) => format!("{n} {unit}"),
            (Kind::Choice(opts), Val::Text(s)) => opts.iter().find(|(o, _)| o == s).map(|(_, l)| l.to_string()).unwrap_or_else(|| s.clone()),
            (Kind::Text { .. }, Val::Text(s)) if s.is_empty() => "empty".into(),
            (Kind::Text { .. }, Val::Text(s)) => s.clone(),
            (_, Val::Colour(c)) => colour_hex(*c),
            (_, v) => format!("{v:?}"),
        }
    }

    /// What kind of value it takes, in words.
    pub fn kind_text(&self) -> String {
        match self.kind {
            Kind::Bool => "on or off (`true` / `false`)".into(),
            Kind::Int { min, max, unit } => format!("a whole number of {unit}, {min} to {max}"),
            Kind::Text { .. } => "text".into(),
            Kind::Choice(opts) => {
                let list: Vec<String> = opts.iter().map(|(v, l)| format!("`\"{v}\"` ({l})")).collect();
                format!("one of {}", list.join(", "))
            }
            Kind::Colour => "a colour, `\"#rrggbb\"`".into(),
        }
    }
}

/// A workbook setting's value, and why its stored value was rejected.
pub fn workbook_value(wb: &Workbook, s: &Setting) -> (Val, Option<String>) {
    debug_assert_eq!(s.scope, Scope::Workbook);
    s.resolve(wb.settings.get(s.key))
}

/// Sets a workbook setting (`None`: back to the default, which removes it from the file).
pub fn set_workbook_value(wb: &mut Workbook, s: &Setting, v: Option<&Val>) -> Result<(), String> {
    debug_assert_eq!(s.scope, Scope::Workbook);
    match v {
        Some(v) => {
            s.check(v)?;
            wb.settings.insert(s.key.to_string(), s.to_json(v));
        }
        None => {
            wb.settings.remove(s.key);
        }
    }
    Ok(())
}

/// Every stored workbook setting that's declared but invalid: (key, why).
pub fn workbook_problems(wb: &Workbook) -> Vec<(&'static str, String)> {
    SETTINGS.iter().filter(|s| s.scope == Scope::Workbook).filter_map(|s| workbook_value(wb, s).1.map(|why| (s.key, why))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn declarations_are_consistent() {
        for (i, s) in SETTINGS.iter().enumerate() {
            assert!(s.key.split('.').count() == 2 && s.key.split('.').all(|p| !p.is_empty()), "{}: keys are section.name", s.key);
            assert!(SETTINGS[..i].iter().all(|o| o.key != s.key), "{} declared twice", s.key);
            assert!(!s.label.is_empty() && !s.help.is_empty() && !s.section.is_empty());
            // the default is a valid value of its own kind, and survives storing
            let d = s.default_val();
            assert_eq!(s.check(&d), Ok(()), "{}: default rejected", s.key);
            assert_eq!(s.parse(&s.to_json(&d)), Ok(d), "{}: default round trip", s.key);
        }
        assert_eq!(sections(Scope::App), vec!["Autosave", "View", "Charts", "Collaboration"]);
        assert_eq!(sections(Scope::Workbook), vec!["Autosave"]);
    }

    #[test]
    fn values_round_trip() {
        for (s, v) in [
            (AUTOSAVE_ENABLED, Val::Bool(true)),
            (AUTOSAVE_INTERVAL, Val::Int(5)),
            (AUTOSAVE_INTERVAL, Val::Int(3600)),
            (AUTOSAVE_WORKBOOK, Val::Text("never".into())),
            (DISPLAY_NAME, Val::Text("Ada".into())),
            (COLOUR, Val::Colour([0xff, 0x00, 0x7f])),
            (SYNC_SERVER, Val::Text("ws://localhost:3030/sync".into())),
        ] {
            assert_eq!(s.resolve(Some(&s.to_json(&v))), (v, None));
        }
        assert_eq!(COLOUR.to_json(&Val::Colour([0xff, 0, 0x7f])), json!("#ff007f"));
        assert_eq!(COLOUR.parse(&json!("#FF007F")), Ok(Val::Colour([0xff, 0, 0x7f])));
    }

    #[test]
    fn invalid_values_are_reported_and_the_default_used() {
        let cases = [
            (AUTOSAVE_ENABLED, json!("yes"), "expected true or false, found \"yes\""),
            (AUTOSAVE_INTERVAL, json!(2), "must be from 5 to 3600 seconds, found 2"),
            (AUTOSAVE_INTERVAL, json!(2.5), "expected a whole number of seconds, found 2.5"),
            (AUTOSAVE_WORKBOOK, json!("sometimes"), "expected one of \"app\", \"always\", \"never\", found \"sometimes\""),
            (COLOUR, json!("green"), "expected a colour like \"#0ea572\", found \"green\""),
            (SYNC_SERVER, json!("https://sync.automerge.org"), "must start with wss:// or ws://"),
            (SYNC_SERVER, json!("wss://"), "needs a server name after the ://"),
            (SYNC_SERVER, json!("wss://host:http"), "\"http\" isn't a port number"),
            (SYNC_SERVER, json!(["wss://a"]), "expected text in quotes, found a list"),
            (DISPLAY_NAME, json!("two\nlines"), "must be on one line"),
        ];
        for (s, raw, why) in cases {
            assert_eq!(s.resolve(Some(&raw)), (s.default_val(), Some(why.to_string())), "{}", s.key);
        }
        assert_eq!(SYNC_SERVER.parse(&json!("wss://[::1]:3030")), Ok(Val::Text("wss://[::1]:3030".into())));
    }

    #[test]
    fn workbook_settings_keep_unknown_and_invalid_values() {
        let mut wb = Workbook::empty();
        wb.settings.insert("future.thing".into(), json!({"a": 1}));
        wb.settings.insert(AUTOSAVE_WORKBOOK.key.into(), json!("sometimes"));
        assert_eq!(workbook_value(&wb, AUTOSAVE_WORKBOOK).0, Val::Text("app".into()));
        assert_eq!(workbook_problems(&wb).len(), 1);
        assert_eq!(unknown_keys(Scope::Workbook, wb.settings.keys().map(String::as_str)), vec!["future.thing"]);
        // an app setting in a workbook file isn't a workbook setting
        assert_eq!(unknown_keys(Scope::Workbook, [AUTOSAVE_ENABLED.key]), vec![AUTOSAVE_ENABLED.key]);
        // a rejected value is refused, the stored one untouched
        assert!(set_workbook_value(&mut wb, AUTOSAVE_WORKBOOK, Some(&Val::Text("nope".into()))).is_err());
        assert_eq!(wb.settings[AUTOSAVE_WORKBOOK.key], json!("sometimes"));
        set_workbook_value(&mut wb, AUTOSAVE_WORKBOOK, Some(&Val::Text("never".into()))).unwrap();
        // through the file format and back
        let s = serde_json::to_string(&wb).unwrap();
        let back: Workbook = serde_json::from_str(&s).unwrap();
        assert_eq!(workbook_value(&back, AUTOSAVE_WORKBOOK), (Val::Text("never".into()), None));
        assert_eq!(back.settings["future.thing"], json!({"a": 1}));
        // reset removes it from the file
        set_workbook_value(&mut wb, AUTOSAVE_WORKBOOK, None).unwrap();
        assert!(!wb.settings.contains_key(AUTOSAVE_WORKBOOK.key));
    }

    #[test]
    fn workbooks_without_settings_load_and_save_without_them() {
        let wb = crate::stdlib::default_workbook();
        let mut v: Json = serde_json::to_value(&wb).unwrap();
        assert!(v.get("settings").is_none(), "no settings key unless something is set");
        v.as_object_mut().unwrap().remove("settings");
        let back: Workbook = serde_json::from_value(v).unwrap();
        assert!(back.settings.is_empty());
    }
}
