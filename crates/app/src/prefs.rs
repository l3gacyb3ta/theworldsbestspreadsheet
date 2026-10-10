//! App preferences: the app-scope settings declared in `wbs_core::settings`,
//! kept in `settings.toml` in the platform config dir (`~/Library/Application
//! Support/wbs` on macOS, `$XDG_CONFIG_HOME/wbs` on Linux), separate from
//! eframe's window-state storage.
//!
//! The file is edited in place with toml_edit, so comments, order and keys this
//! version doesn't know survive a rewrite. A stored value that doesn't pass its
//! declaration is reported and the default used, never rewritten. The file is
//! only written when a setting is changed, and never if it couldn't be read.

use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, Item, Table, Value};
use wbs_core::settings::{self, Scope, Setting, Val};

const HEADER: &str = "# the world's best spreadsheet: app preferences.\n# Every setting is described in Help ▸ Settings. Remove a line to go back to its default.\n\n";

pub struct Prefs {
    /// `None`: kept in memory only.
    path: Option<PathBuf>,
    doc: DocumentMut,
    /// The file exists but couldn't be read: changes apply but it's not overwritten.
    broken: Option<String>,
    write_error: Option<String>,
}

impl Prefs {
    /// Defaults, stored nowhere (tests, or a machine without a config dir).
    pub fn in_memory() -> Prefs {
        Prefs { path: None, doc: DocumentMut::new(), broken: None, write_error: None }
    }

    /// `settings.toml` in the platform config dir.
    pub fn default_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|d| d.config_dir().join("wbs").join("settings.toml"))
    }

    /// Reads `path`; a missing file is all defaults (and isn't created until something changes).
    pub fn load(path: PathBuf) -> Prefs {
        let mut p = Prefs { path: Some(path.clone()), ..Prefs::in_memory() };
        match std::fs::read_to_string(&path) {
            Ok(s) => match s.parse::<DocumentMut>() {
                Ok(doc) => p.doc = doc,
                Err(e) => p.broken = Some(format!("couldn't read {}: {}", path.display(), e.to_string().trim())),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => p.broken = Some(format!("couldn't read {}: {e}", path.display())),
        }
        p
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Why the file couldn't be read or the last change couldn't be written.
    pub fn file_error(&self) -> Option<String> {
        match (&self.broken, &self.write_error) {
            (Some(b), _) => Some(format!("{b}. Changes apply until you quit, but aren't saved, so the file isn't overwritten.")),
            (None, Some(w)) => Some(w.clone()),
            (None, None) => None,
        }
    }

    /// What's stored for `s`, as JSON (`None`: not set), or why it can't be read.
    fn raw(&self, s: &Setting) -> Result<Option<Json>, String> {
        let (section, name) = s.key.split_once('.').unwrap();
        let Some(sec) = self.doc.get(section) else { return Ok(None) };
        let Some(t) = sec.as_table_like() else { return Err(format!("`{section}` in the file should be a [{section}] table")) };
        match t.get(name) {
            None | Some(Item::None) => Ok(None),
            Some(Item::Value(v)) => to_json(v).map(Some),
            Some(Item::Table(_) | Item::ArrayOfTables(_)) => Ok(Some(Json::Object(Default::default()))),
        }
    }

    /// The value to use for `s`, and why the stored one was rejected.
    pub fn get(&self, s: &Setting) -> (Val, Option<String>) {
        debug_assert_eq!(s.scope, Scope::App);
        match self.raw(s) {
            Ok(raw) => s.resolve(raw.as_ref()),
            Err(why) => (s.default_val(), Some(why)),
        }
    }

    pub fn val(&self, s: &Setting) -> Val {
        self.get(s).0
    }

    /// Whether the file has an entry for `s` (valid or not).
    pub fn is_set(&self, s: &Setting) -> bool {
        !matches!(self.raw(s), Ok(None))
    }

    /// Every declared setting whose stored value is rejected: (key, why).
    pub fn problems(&self) -> Vec<(&'static str, String)> {
        settings::SETTINGS.iter().filter(|s| s.scope == Scope::App).filter_map(|s| self.get(s).1.map(|why| (s.key, why))).collect()
    }

    /// Keys in the file that no app setting declares (kept as they are).
    pub fn unknown_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for (k, item) in self.doc.iter() {
            match item.as_table_like() {
                Some(t) => keys.extend(t.iter().map(|(n, _)| format!("{k}.{n}"))),
                None => keys.push(k.to_string()),
            }
        }
        settings::unknown_keys(Scope::App, keys.iter().map(String::as_str))
    }

    /// Sets `s` (`None`: back to the default, which removes it from the file) and writes the file.
    pub fn set(&mut self, s: &Setting, v: Option<&Val>) -> Result<(), String> {
        debug_assert_eq!(s.scope, Scope::App);
        let (section, name) = s.key.split_once('.').unwrap();
        let root = self.doc.as_table_mut();
        match v {
            Some(v) => {
                s.check(v)?;
                if !root.contains_key(section) {
                    root.insert(section, Item::Table(Table::new()));
                }
                let t = root[section].as_table_like_mut().ok_or_else(|| format!("can't save: `{section}` in the file should be a [{section}] table"))?;
                let item = match s.to_json(v) {
                    Json::Bool(b) => toml_edit::value(b),
                    Json::Number(n) => toml_edit::value(n.as_i64().unwrap()),
                    Json::String(t) => toml_edit::value(t),
                    other => unreachable!("settings are stored as scalars, not {other}"),
                };
                // keep a comment written after the old value
                let decor = t.get(name).and_then(|i| i.as_value()).map(|v| v.decor().clone());
                t.insert(name, item);
                if let (Some(d), Some(v)) = (decor, t.get_mut(name).and_then(|i| i.as_value_mut())) {
                    *v.decor_mut() = d;
                }
            }
            None => {
                if let Some(t) = root.get_mut(section).and_then(|i| i.as_table_like_mut()) {
                    t.remove(name);
                }
            }
        }
        self.write()
    }

    fn write(&mut self) -> Result<(), String> {
        let Some(path) = &self.path else { return Ok(()) };
        if self.broken.is_some() {
            return Ok(());
        }
        let mut text = self.doc.to_string();
        if !path.exists() {
            text = format!("{HEADER}{text}");
        }
        let tmp = path.with_extension("toml.tmp");
        let res = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&tmp, &text)).and_then(|_| std::fs::rename(&tmp, path));
        self.write_error = res.err().map(|e| format!("couldn't save settings to {}: {e}", path.display()));
        if let Some(e) = &self.write_error {
            return Err(e.clone());
        }
        // reread what's on disk so the header comment is part of the document from now on
        if let Ok(doc) = text.parse() {
            self.doc = doc;
        }
        Ok(())
    }
}

fn to_json(v: &Value) -> Result<Json, String> {
    Ok(match v {
        Value::String(s) => Json::String(s.value().clone()),
        Value::Integer(i) => Json::from(*i.value()),
        Value::Float(f) => serde_json::Number::from_f64(*f.value()).map(Json::Number).ok_or_else(|| format!("{} isn't a number", f.value()))?,
        Value::Boolean(b) => Json::Bool(*b.value()),
        Value::Datetime(d) => return Err(format!("found a date ({}), which no setting takes", d.value())),
        Value::Array(a) => Json::Array(a.iter().map(to_json).collect::<Result<_, _>>()?),
        Value::InlineTable(t) => Json::Object(t.iter().map(|(k, v)| Ok((k.to_string(), to_json(v)?))).collect::<Result<_, String>>()?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wbs_core::settings::{AUTOSAVE_ENABLED, AUTOSAVE_INTERVAL, COLOUR, DISPLAY_NAME, SYNC_SERVER};

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wbs-prefs-tests-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn defaults_without_a_file_and_nothing_written() {
        let p = tmp("missing.toml");
        let prefs = Prefs::load(p.clone());
        assert_eq!(prefs.val(AUTOSAVE_ENABLED), Val::Bool(true));
        assert_eq!(prefs.val(AUTOSAVE_INTERVAL), Val::Int(60));
        assert_eq!(prefs.val(SYNC_SERVER), Val::Text("wss://sync.automerge.org".into()));
        assert!(prefs.problems().is_empty() && prefs.unknown_keys().is_empty() && prefs.file_error().is_none());
        assert!(!p.exists(), "reading never creates the file");
    }

    #[test]
    fn round_trip_through_the_file() {
        let p = tmp("round_trip.toml");
        let mut prefs = Prefs::load(p.clone());
        prefs.set(AUTOSAVE_ENABLED, Some(&Val::Bool(false))).unwrap();
        prefs.set(AUTOSAVE_INTERVAL, Some(&Val::Int(30))).unwrap();
        prefs.set(COLOUR, Some(&Val::Colour([1, 2, 3]))).unwrap();
        prefs.set(DISPLAY_NAME, Some(&Val::Text("Ada".into()))).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("# the world's best spreadsheet"), "{text}");
        assert!(text.contains("[autosave]\nenabled = false\ninterval_seconds = 30\n"), "{text}");
        assert!(text.contains("colour = \"#010203\""), "{text}");
        let back = Prefs::load(p.clone());
        assert_eq!(back.val(AUTOSAVE_ENABLED), Val::Bool(false));
        assert_eq!(back.val(AUTOSAVE_INTERVAL), Val::Int(30));
        assert_eq!(back.val(COLOUR), Val::Colour([1, 2, 3]));
        assert_eq!(back.val(DISPLAY_NAME), Val::Text("Ada".into()));
        // an invalid value is refused and nothing changes
        let mut prefs = back;
        assert!(prefs.set(AUTOSAVE_INTERVAL, Some(&Val::Int(1))).is_err());
        assert_eq!(Prefs::load(p.clone()).val(AUTOSAVE_INTERVAL), Val::Int(30));
        // reset removes the line
        prefs.set(AUTOSAVE_INTERVAL, None).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("interval_seconds"));
        assert!(!prefs.is_set(AUTOSAVE_INTERVAL));
    }

    #[test]
    fn invalid_values_are_reported_kept_and_the_default_used() {
        let p = tmp("invalid.toml");
        let src = "# mine\n[autosave]\nenabled = \"yes\" # typo\ninterval_seconds = 2\n\n[sync]\nserver_url = \"https://example.com\"\n";
        std::fs::write(&p, src).unwrap();
        let mut prefs = Prefs::load(p.clone());
        assert_eq!(prefs.get(AUTOSAVE_ENABLED), (Val::Bool(true), Some("expected true or false, found \"yes\"".into())));
        assert_eq!(prefs.get(AUTOSAVE_INTERVAL), (Val::Int(60), Some("must be from 5 to 3600 seconds, found 2".into())));
        assert_eq!(prefs.get(SYNC_SERVER).1.as_deref(), Some("must start with wss:// or ws://"));
        assert_eq!(prefs.problems().len(), 3);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), src, "loading doesn't touch the file");
        // fixing one leaves the others, and the comments, as they were
        prefs.set(AUTOSAVE_ENABLED, Some(&Val::Bool(true))).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text, src.replace("enabled = \"yes\" # typo", "enabled = true # typo"));
        // a section that isn't a table
        std::fs::write(&p, "autosave = 3\n").unwrap();
        let mut prefs = Prefs::load(p.clone());
        assert_eq!(prefs.get(AUTOSAVE_ENABLED).1.as_deref(), Some("`autosave` in the file should be a [autosave] table"));
        assert!(prefs.set(AUTOSAVE_ENABLED, Some(&Val::Bool(true))).is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "autosave = 3\n");
    }

    #[test]
    fn unknown_keys_are_kept() {
        let p = tmp("unknown.toml");
        std::fs::write(&p, "future = 1\n\n[autosave]\nenabled = true\nsmart = \"maybe\"\n\n[plugins]\nlist = [\"a\", \"b\"]\n").unwrap();
        let mut prefs = Prefs::load(p.clone());
        assert_eq!(prefs.unknown_keys(), vec!["future", "autosave.smart", "plugins.list"]);
        prefs.set(AUTOSAVE_INTERVAL, Some(&Val::Int(10))).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert_eq!(text, "future = 1\n\n[autosave]\nenabled = true\nsmart = \"maybe\"\ninterval_seconds = 10\n\n[plugins]\nlist = [\"a\", \"b\"]\n");
        // dotted keys work as well as tables
        std::fs::write(&p, "autosave.enabled = false\n").unwrap();
        assert_eq!(Prefs::load(p).val(AUTOSAVE_ENABLED), Val::Bool(false));
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let p = tmp("broken.toml");
        std::fs::write(&p, "[autosave\nenabled = true\n").unwrap();
        let mut prefs = Prefs::load(p.clone());
        assert!(prefs.file_error().unwrap().contains("couldn't read"));
        prefs.set(AUTOSAVE_ENABLED, Some(&Val::Bool(true))).unwrap();
        assert_eq!(prefs.val(AUTOSAVE_ENABLED), Val::Bool(true), "applies for this session");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[autosave\nenabled = true\n");
    }
}
