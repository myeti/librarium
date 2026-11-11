//! The native menu bar (macOS).
//!
//! The menu is described as data ([`MENU_BAR`]: one table of [`Entry`]s per
//! submenu) and turned into a Tauri [`Menu`] by [`build`], in English or
//! French ([`Lang`]). The table can be checked without a window (see the
//! tests below).
//!
//! **macOS only.** The menu is installed at startup ([`install`], English)
//! and rebuilt in the user's language by [`set_menu_language`], which the
//! frontend calls at startup and whenever the language changes. On Windows
//! and Linux a menu bar would add a native strip to the top of the window
//! and change the app's look, so there is none: the Keyboard shortcuts
//! dialog lists the same commands there, and `set_menu_language` does
//! nothing. Installing our own menu replaces Tauri's default macOS menu,
//! which is why the Edit and Window menus are here too (copy/paste, ⌘M,
//! ⌘W… keep working).
//!
//! **Events.** Choosing one of our own items ([`Entry::Custom`]) emits one
//! app-wide event, [`COMMAND_EVENT`] (`menu-command`), whose payload is the
//! item's id as a JSON string (`"library.import"`). The frontend routes it
//! through its shortcuts store, as if the shortcut had been pressed. Only
//! ids of this table are forwarded. Predefined items (Copy, Hide, Quit…)
//! are handled by the OS and emit nothing.
//!
//! **Accelerators.** The accelerators shown here mirror
//! `src/constants/shortcuts.ts` (`Mod` there is `CmdOrCtrl` here); a
//! frontend test reads this file and checks them. So each custom entry is
//! written on one line, as a call to the `custom` helper with four
//! arguments: the id, the English label, the French label (all three
//! string literals) and the accelerator (`None`, or `Some` with a string
//! literal). Keep that shape; the `custom_entries_are_written_on_one_line`
//! test enforces it.

// Outside macOS the menu is never built (see above), so the table and the
// builder are only used by the tests there; they stay compiled everywhere so
// that the tests run on every platform.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use tauri::menu::{
    Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID, WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Emitter, Runtime};

use crate::error::AppError;
use crate::logging::log;

/// The event emitted when a custom item is chosen; the payload is its id.
pub(crate) const COMMAND_EVENT: &str = "menu-command";

/// The language of the menu's labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Lang {
    En,
    Fr,
}

impl Lang {
    /// The language for a frontend language code: `fr` (or a regional
    /// variant, `fr-CA`) is French, anything else English.
    pub(crate) fn from_code(code: &str) -> Self {
        let primary = code.trim().split(['-', '_']).next().unwrap_or_default();
        if primary.eq_ignore_ascii_case("fr") {
            Self::Fr
        } else {
            Self::En
        }
    }

    fn pick(self, en: &'static str, fr: &'static str) -> &'static str {
        match self {
            Self::En => en,
            Self::Fr => fr,
        }
    }
}

/// One submenu of the menu bar.
pub(crate) struct SubmenuSpec {
    pub id: &'static str,
    pub title_en: &'static str,
    pub title_fr: &'static str,
    pub entries: &'static [Entry],
}

/// One line of a submenu.
pub(crate) enum Entry {
    /// One of our commands: choosing it emits [`COMMAND_EVENT`] with `id`.
    Custom {
        id: &'static str,
        label_en: &'static str,
        label_fr: &'static str,
        /// `CmdOrCtrl+O` (muda's syntax); mirrors `src/constants/shortcuts.ts`.
        accelerator: Option<&'static str>,
    },
    Separator,
    /// A standard item the OS implements (and gives its usual shortcut).
    Predefined(Predefined),
}

/// The standard items used, each with its English and French label
/// ([`Predefined::label`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Predefined {
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
    CloseWindow,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Fullscreen,
    Minimize,
    Maximize,
}

impl Predefined {
    /// `(English, French)`.
    const fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Services => ("Services", "Services"),
            Self::Hide => ("Hide Librarium", "Masquer Librarium"),
            Self::HideOthers => ("Hide Others", "Masquer les autres"),
            Self::ShowAll => ("Show All", "Tout afficher"),
            Self::Quit => ("Quit Librarium", "Quitter Librarium"),
            Self::CloseWindow => ("Close Window", "Fermer la fenêtre"),
            Self::Undo => ("Undo", "Annuler"),
            Self::Redo => ("Redo", "Rétablir"),
            Self::Cut => ("Cut", "Couper"),
            Self::Copy => ("Copy", "Copier"),
            Self::Paste => ("Paste", "Coller"),
            Self::SelectAll => ("Select All", "Tout sélectionner"),
            Self::Fullscreen => ("Enter Full Screen", "Plein écran"),
            Self::Minimize => ("Minimize", "Réduire"),
            Self::Maximize => ("Zoom", "Zoom"),
        }
    }

    fn label(self, lang: Lang) -> &'static str {
        let (en, fr) = self.labels();
        lang.pick(en, fr)
    }
}

/// A custom entry (see the module docs for how to write one).
const fn custom(
    id: &'static str,
    label_en: &'static str,
    label_fr: &'static str,
    accelerator: Option<&'static str>,
) -> Entry {
    Entry::Custom {
        id,
        label_en,
        label_fr,
        accelerator,
    }
}

const SEPARATOR: Entry = Entry::Separator;

/// A standard item, handled by the OS.
const fn os(item: Predefined) -> Entry {
    Entry::Predefined(item)
}

// ── The menu bar ────────────────────────────────────────────────────────────

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const APP_MENU: &[Entry] = &[
    custom("app.about", "About Librarium", "À propos de Librarium", None),
    SEPARATOR,
    custom("app.settings", "Settings…", "Réglages…", Some("CmdOrCtrl+,")),
    SEPARATOR,
    os(Predefined::Services),
    SEPARATOR,
    os(Predefined::Hide),
    os(Predefined::HideOthers),
    os(Predefined::ShowAll),
    SEPARATOR,
    os(Predefined::Quit),
];

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const FILE_MENU: &[Entry] = &[
    custom("library.import", "Import Books…", "Importer des livres…", Some("CmdOrCtrl+O")),
    custom("library.export", "Export…", "Exporter…", None),
    SEPARATOR,
    os(Predefined::CloseWindow),
];

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const EDIT_MENU: &[Entry] = &[
    os(Predefined::Undo),
    os(Predefined::Redo),
    SEPARATOR,
    os(Predefined::Cut),
    os(Predefined::Copy),
    os(Predefined::Paste),
    os(Predefined::SelectAll),
];

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const VIEW_MENU: &[Entry] = &[
    custom("view.grid", "Grid", "Grille", Some("CmdOrCtrl+1")),
    custom("view.list", "List", "Liste", Some("CmdOrCtrl+2")),
    SEPARATOR,
    custom("view.toggleDetails", "Show or Hide Details", "Afficher ou masquer les détails", Some("CmdOrCtrl+I")),
    custom("library.search", "Search", "Rechercher", Some("CmdOrCtrl+F")),
    SEPARATOR,
    os(Predefined::Fullscreen),
];

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const LIBRARY_MENU: &[Entry] = &[
    custom("library.detectDevice", "Detect E-reader", "Détecter une liseuse", Some("CmdOrCtrl+K")),
    custom("library.findDuplicates", "Find Duplicates…", "Rechercher les doublons…", None),
];

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const WINDOW_MENU: &[Entry] = &[
    os(Predefined::Minimize),
    os(Predefined::Maximize),
];

// One entry per line: src/constants/menu.test.ts (and the test below)
// read this file as text to check the accelerators.
#[rustfmt::skip]
const HELP_MENU: &[Entry] = &[
    custom("help.shortcuts", "Keyboard Shortcuts", "Raccourcis clavier", Some("CmdOrCtrl+/")),
    custom("help.logs", "Open Log Folder", "Ouvrir le dossier des journaux", None),
    custom("help.privacy", "Privacy", "Confidentialité", None),
];

/// The whole menu bar, left to right. The Window and Help submenus carry
/// Tauri's ids so that macOS treats them as such (window list in Window,
/// search field in Help).
pub(crate) const MENU_BAR: &[SubmenuSpec] = &[
    SubmenuSpec {
        id: "menu.app",
        title_en: "Librarium",
        title_fr: "Librarium",
        entries: APP_MENU,
    },
    SubmenuSpec {
        id: "menu.file",
        title_en: "File",
        title_fr: "Fichier",
        entries: FILE_MENU,
    },
    SubmenuSpec {
        id: "menu.edit",
        title_en: "Edit",
        title_fr: "Édition",
        entries: EDIT_MENU,
    },
    SubmenuSpec {
        id: "menu.view",
        title_en: "View",
        title_fr: "Affichage",
        entries: VIEW_MENU,
    },
    SubmenuSpec {
        id: "menu.library",
        title_en: "Library",
        title_fr: "Bibliothèque",
        entries: LIBRARY_MENU,
    },
    SubmenuSpec {
        id: WINDOW_SUBMENU_ID,
        title_en: "Window",
        title_fr: "Fenêtre",
        entries: WINDOW_MENU,
    },
    SubmenuSpec {
        id: HELP_SUBMENU_ID,
        title_en: "Help",
        title_fr: "Aide",
        entries: HELP_MENU,
    },
];

/// The ids of every custom entry: the possible payloads of [`COMMAND_EVENT`].
pub(crate) fn command_ids() -> impl Iterator<Item = &'static str> {
    MENU_BAR
        .iter()
        .flat_map(|submenu| submenu.entries)
        .filter_map(|entry| match entry {
            Entry::Custom { id, .. } => Some(*id),
            _ => None,
        })
}

/// Whether `id` is one of our commands (not a predefined item or a submenu).
pub(crate) fn is_command(id: &str) -> bool {
    command_ids().any(|c| c == id)
}

// ── Building and installing ─────────────────────────────────────────────────

/// The menu bar described by [`MENU_BAR`], labelled in `lang`. Tauri creates
/// each item on the main thread (and waits for it from another thread).
pub(crate) fn build<R: Runtime>(app: &AppHandle<R>, lang: Lang) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    for spec in MENU_BAR {
        let submenu =
            Submenu::with_id(app, spec.id, lang.pick(spec.title_en, spec.title_fr), true)?;
        for entry in spec.entries {
            match *entry {
                Entry::Custom {
                    id,
                    label_en,
                    label_fr,
                    accelerator,
                } => {
                    submenu.append(&MenuItem::with_id(
                        app,
                        id,
                        lang.pick(label_en, label_fr),
                        true,
                        accelerator,
                    )?)?;
                }
                Entry::Separator => submenu.append(&PredefinedMenuItem::separator(app)?)?,
                Entry::Predefined(item) => submenu.append(&predefined_item(app, item, lang)?)?,
            }
        }
        menu.append(&submenu)?;
    }
    Ok(menu)
}

fn predefined_item<R: Runtime>(
    app: &AppHandle<R>,
    item: Predefined,
    lang: Lang,
) -> tauri::Result<PredefinedMenuItem<R>> {
    let text = Some(item.label(lang));
    match item {
        Predefined::Services => PredefinedMenuItem::services(app, text),
        Predefined::Hide => PredefinedMenuItem::hide(app, text),
        Predefined::HideOthers => PredefinedMenuItem::hide_others(app, text),
        Predefined::ShowAll => PredefinedMenuItem::show_all(app, text),
        Predefined::Quit => PredefinedMenuItem::quit(app, text),
        Predefined::CloseWindow => PredefinedMenuItem::close_window(app, text),
        Predefined::Undo => PredefinedMenuItem::undo(app, text),
        Predefined::Redo => PredefinedMenuItem::redo(app, text),
        Predefined::Cut => PredefinedMenuItem::cut(app, text),
        Predefined::Copy => PredefinedMenuItem::copy(app, text),
        Predefined::Paste => PredefinedMenuItem::paste(app, text),
        Predefined::SelectAll => PredefinedMenuItem::select_all(app, text),
        Predefined::Fullscreen => PredefinedMenuItem::fullscreen(app, text),
        Predefined::Minimize => PredefinedMenuItem::minimize(app, text),
        Predefined::Maximize => PredefinedMenuItem::maximize(app, text),
    }
}

/// Build the menu bar in `lang` and make it the app's menu (replacing the
/// previous one). Call it on the main thread: every step then runs inline
/// instead of being sent to the main thread one item at a time.
fn install_menu(app: &AppHandle, lang: Lang) -> tauri::Result<()> {
    app.set_menu(build(app, lang)?)?;
    Ok(())
}

/// Forward a click on one of our items to the frontend as [`COMMAND_EVENT`].
fn forward_command(app: &AppHandle, event: MenuEvent) {
    let id: &str = event.id().as_ref();
    if !is_command(id) {
        return;
    }
    if let Err(e) = app.emit(COMMAND_EVENT, id) {
        log(
            app,
            format!("[menu] ERROR — could not send the menu command {id}: {e}"),
        );
    }
}

/// At startup (in `setup`, on the main thread): install the English menu bar
/// and forward its commands. A menu that can't be built is logged and
/// leaves Tauri's default menu in place; the app works without ours.
#[cfg(target_os = "macos")]
pub(crate) fn install(app: &AppHandle) {
    app.on_menu_event(forward_command);
    if let Err(e) = install_menu(app, Lang::En) {
        log(
            app,
            format!("[menu] ERROR — could not build the menu bar: {e}"),
        );
    }
}

/// Rebuild the menu bar in `lang` (`en`, `fr`; anything else is English).
/// macOS only; elsewhere there is no menu bar and this does nothing. The
/// menu is built on the main thread, as AppKit requires; the command waits
/// for it without blocking an async worker.
#[tauri::command]
pub async fn set_menu_language(app: AppHandle, lang: String) -> Result<(), AppError> {
    #[cfg(target_os = "macos")]
    {
        let lang = Lang::from_code(&lang);
        let (tx, rx) = tokio::sync::oneshot::channel();
        let handle = app.clone();
        app.run_on_main_thread(move || {
            let _ = tx.send(install_menu(&handle, lang).map_err(AppError::from));
        })?;
        let result = rx
            .await
            .unwrap_or_else(|_| Err(AppError::Internal("the menu bar update never ran".into())));
        if let Err(e) = &result {
            log(
                &app,
                format!("[set_menu_language] ERROR ({}) — {e}", e.code()),
            );
        }
        result
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, lang);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const SOURCE: &str = include_str!("menu.rs");
    const IPC_DOC: &str = include_str!("../../docs/IPC.md");

    struct CustomEntry {
        id: &'static str,
        label_en: &'static str,
        label_fr: &'static str,
        accelerator: Option<&'static str>,
    }

    fn custom_entries() -> Vec<CustomEntry> {
        MENU_BAR
            .iter()
            .flat_map(|submenu| submenu.entries)
            .filter_map(|entry| match *entry {
                Entry::Custom {
                    id,
                    label_en,
                    label_fr,
                    accelerator,
                } => Some(CustomEntry {
                    id,
                    label_en,
                    label_fr,
                    accelerator,
                }),
                _ => None,
            })
            .collect()
    }

    fn predefined_entries() -> Vec<Predefined> {
        MENU_BAR
            .iter()
            .flat_map(|submenu| submenu.entries)
            .filter_map(|entry| match *entry {
                Entry::Predefined(item) => Some(item),
                _ => None,
            })
            .collect()
    }

    /// Whether muda would parse `accelerator`. Its parser
    /// (`muda::accelerator::Accelerator: FromStr`) isn't reachable from this
    /// crate: Tauri re-exports only `MenuId` from muda, and `MenuItem::with_id`
    /// silently drops an accelerator it can't parse. So this checks the
    /// subset of muda's syntax this menu uses: `CmdOrCtrl` first (the
    /// frontend's `Mod`), optionally `Shift`/`Alt`, then one key muda names
    /// by its character: a capital letter, a digit or a punctuation key
    /// (comma, slash, period…).
    fn muda_accepts(accelerator: &str) -> bool {
        let mut parts: Vec<&str> = accelerator.split('+').collect();
        let key = parts.pop().unwrap_or_default();
        let modifiers_ok = parts.first() == Some(&"CmdOrCtrl")
            && parts[1..].iter().all(|m| matches!(*m, "Shift" | "Alt"))
            && parts.iter().collect::<HashSet<_>>().len() == parts.len();
        let key_ok = matches!(
            key.as_bytes(),
            [b'A'..=b'Z'
            | b'0'..=b'9'
            | b','
            | b'.'
            | b'/'
            | b';'
            | b'\''
            | b'-'
            | b'='
            | b'['
            | b']'
            | b'\\'
            | b'`']
        );
        modifiers_ok && key_ok
    }

    #[test]
    fn ids_are_unique() {
        let mut seen = HashSet::new();
        for spec in MENU_BAR {
            assert!(seen.insert(spec.id), "duplicate submenu id {}", spec.id);
        }
        for entry in custom_entries() {
            assert!(seen.insert(entry.id), "duplicate id {}", entry.id);
        }
        let predefined = predefined_entries();
        assert_eq!(
            predefined.iter().collect::<HashSet<_>>().len(),
            predefined.len(),
            "a predefined item appears twice"
        );
    }

    #[test]
    fn every_command_id_is_documented() {
        let ids: Vec<_> = command_ids().collect();
        assert_eq!(ids.len(), 13, "{ids:?}");
        for id in ids {
            // The frontend test matches ids with `[\w.]+`.
            assert!(
                id.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') && id.contains('.'),
                "{id}"
            );
            assert!(
                IPC_DOC.contains(&format!("`{id}`")),
                "{id} is not documented in docs/IPC.md (Menu bar)"
            );
        }
        assert!(
            IPC_DOC.contains(&format!("`{COMMAND_EVENT}`")),
            "the event is not documented in docs/IPC.md"
        );
    }

    #[test]
    fn only_command_ids_are_forwarded() {
        assert!(is_command("library.import"));
        assert!(is_command("app.about"));
        assert!(!is_command("menu.file"), "a submenu");
        assert!(!is_command(WINDOW_SUBMENU_ID));
        assert!(
            !is_command("3"),
            "muda's ids for predefined items are counters"
        );
        assert!(!is_command(""));
        assert!(
            !is_command("library.delete"),
            "a shortcut that isn't in the menu"
        );
    }

    #[test]
    fn accelerators_parse_and_are_unique() {
        let mut seen = HashSet::new();
        for entry in custom_entries() {
            let Some(accelerator) = entry.accelerator else {
                continue;
            };
            assert!(
                muda_accepts(accelerator),
                "{}: muda can't parse {accelerator:?}",
                entry.id
            );
            assert!(seen.insert(accelerator), "{accelerator} is used twice");
        }
        assert!(seen.len() >= 8, "{seen:?}");
        // The predefined items' own shortcuts (macOS): ours must not take them.
        for taken in ["A", "C", "H", "M", "Q", "V", "W", "X", "Z"] {
            assert!(
                !seen.contains(format!("CmdOrCtrl+{taken}").as_str()),
                "⌘{taken} belongs to a standard item"
            );
        }
        // The checker itself.
        for good in [
            "CmdOrCtrl+O",
            "CmdOrCtrl+,",
            "CmdOrCtrl+/",
            "CmdOrCtrl+1",
            "CmdOrCtrl+Shift+K",
        ] {
            assert!(muda_accepts(good), "{good}");
        }
        for bad in [
            "Mod+O",
            "CmdOrCtrl+",
            "CmdOrCtrl+o",
            "O",
            "CmdOrCtrl+Return",
            "Shift+CmdOrCtrl+K",
            "CmdOrCtrl+Hyper+K",
        ] {
            assert!(!muda_accepts(bad), "{bad}");
        }
    }

    #[test]
    fn labels_are_translated() {
        // The same word in both languages on purpose.
        let same = ["Librarium", "Services", "Zoom"];
        let mut pairs: Vec<(&str, &str)> =
            MENU_BAR.iter().map(|s| (s.title_en, s.title_fr)).collect();
        pairs.extend(custom_entries().iter().map(|e| (e.label_en, e.label_fr)));
        pairs.extend(predefined_entries().iter().map(|p| p.labels()));
        for (en, fr) in pairs {
            assert!(
                !en.trim().is_empty() && !fr.trim().is_empty(),
                "empty label ({en:?} / {fr:?})"
            );
            // A quote would break the frontend test's pattern.
            assert!(!en.contains('"') && !fr.contains('"'), "{en} / {fr}");
            if !same.contains(&en) {
                assert_ne!(en, fr, "untranslated label");
            }
        }
        assert_eq!(Predefined::Quit.label(Lang::Fr), "Quitter Librarium");
        assert_eq!(Predefined::Quit.label(Lang::En), "Quit Librarium");
    }

    #[test]
    fn custom_entries_are_written_on_one_line() {
        // What the frontend test matches (see the module docs).
        let entries = custom_entries();
        for e in &entries {
            let accelerator = match e.accelerator {
                Some(a) => format!("Some(\"{a}\")"),
                None => "None".to_string(),
            };
            let line = format!(
                "custom(\"{}\", \"{}\", \"{}\", {accelerator}),",
                e.id, e.label_en, e.label_fr
            );
            assert!(
                SOURCE.contains(&line),
                "write the {} entry as `{line}`",
                e.id
            );
        }
        // Nothing else in this file looks like an entry to that test.
        assert_eq!(SOURCE.matches("custom(\"").count(), entries.len());
    }

    #[test]
    fn language_codes() {
        assert_eq!(Lang::from_code("fr"), Lang::Fr);
        assert_eq!(Lang::from_code("FR"), Lang::Fr);
        assert_eq!(Lang::from_code("fr-CA"), Lang::Fr);
        assert_eq!(Lang::from_code("fr_FR"), Lang::Fr);
        assert_eq!(Lang::from_code("en"), Lang::En);
        assert_eq!(Lang::from_code("de"), Lang::En);
        assert_eq!(Lang::from_code("free"), Lang::En);
        assert_eq!(Lang::from_code(""), Lang::En);
        assert_eq!(Lang::Fr.pick("Grid", "Grille"), "Grille");
    }
}
