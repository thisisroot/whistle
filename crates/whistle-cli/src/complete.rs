//! Tab completion of app names, for shells set up with
//! `source <(COMPLETE=bash whistle)` (or zsh, fish).

use std::ffi::OsStr;

use clap_complete::engine::CompletionCandidate;
use whistle_core::index::Index;
use whistle_core::paths::Paths;
use whistle_core::state::State;

/// Most candidates offered at once; more is noise.
const LIMIT: usize = 50;

/// Apps in the catalog, by the name to type (`visual-studio-code`), or by id
/// once what's typed has a dot, like one. Popular apps first, for shells that keep
/// the order.
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
    let mut found: Vec<(u16, String, &str)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for app in &index.apps {
        let slug = app.slug();
        let word = if slug.starts_with(&typed) {
            slug
        } else if typed.contains('.') && app.id.to_lowercase().starts_with(&typed) {
            app.id.clone()
        } else {
            continue;
        };
        // Apps sharing a name complete once; `whistle` asks which one.
        if seen.insert(word.clone()) {
            found.push((app.popularity, word, &app.summary));
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    found
        .into_iter()
        .take(LIMIT)
        .map(|(_, word, summary)| {
            CompletionCandidate::new(word).help(Some(summary.to_owned().into()))
        })
        .collect()
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
