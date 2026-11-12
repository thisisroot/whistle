//! Tab completion of app names, for shells set up with
//! `source <(COMPLETE=bash whistle)` (or zsh, fish).

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;
use whistle_core::index::Index;
use whistle_core::paths::Paths;
use whistle_core::state::State;

/// Most candidates offered at once; more is noise.
const LIMIT: usize = 200;

/// Apps in the catalog: their one-word names (which `whistle` accepts in any
/// case) and their ids.
pub fn catalog_apps(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Ok(paths) = Paths::detect() else {
        return Vec::new();
    };
    let Ok((index, _)) = Index::load(None, &paths) else {
        return Vec::new();
    };
    let typed = current.to_lowercase();
    let mut found: Vec<CompletionCandidate> = Vec::new();
    let mut names = std::collections::HashSet::new();
    for app in &index.apps {
        let name = app.name.to_lowercase();
        if !name.contains(char::is_whitespace) && name.starts_with(&typed) {
            // Apps sharing a name complete once; `whistle` asks which one.
            if names.insert(name.clone()) {
                found.push(CompletionCandidate::new(name).help(Some(app.summary.clone().into())));
            }
        } else if app.id.to_lowercase().starts_with(&typed) {
            found.push(CompletionCandidate::new(&app.id).help(Some(app.name.clone().into())));
        }
        if found.len() >= LIMIT {
            break;
        }
    }
    found
}

/// Apps Whistle installed, for `remove` and `upgrade`.
pub fn installed_apps(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Some(state) = Paths::detect()
        .ok()
        .and_then(|paths| State::load(&paths.state_file()).ok())
    else {
        return Vec::new();
    };
    state
        .installed
        .iter()
        .filter(|i| i.app_id.starts_with(current))
        .map(|i| CompletionCandidate::new(&i.app_id).help(Some(i.app_name.clone().into())))
        .collect()
}
