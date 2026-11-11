//! Recipe types, mirroring `recipe.schema.json` (v1) in whistle-recipes.
//!
//! The recipes repo and the server validate recipes against the schema. The
//! client parses strictly and re-checks the rules that keep a recipe inside its
//! sandbox (paths, environment), so a bad index can't widen what a recipe does.

use std::collections::BTreeMap;
use std::fmt;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    pub schema_version: u32,
    pub id: String,
    pub app: String,
    #[serde(default)]
    pub version_range: Option<String>,
    pub runtime: Runtime,
    #[serde(default)]
    pub maintainers: Vec<String>,
    #[serde(default)]
    pub requires: Requires,
    pub installer: Installer,
    #[serde(default)]
    pub steps: Vec<Step>,
    pub launchers: Vec<Launcher>,
    #[serde(default)]
    pub health_check: Option<HealthCheck>,
    #[serde(default)]
    pub fallback: Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Runtime {
    Wine,
    Proton,
    Vm,
    Wsl,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wine => "wine",
            Self::Proton => "proton",
            Self::Vm => "vm",
            Self::Wsl => "wsl",
        }
    }
}

impl fmt::Display for Runtime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    pub wine: Option<String>,
    pub proton: Option<String>,
    pub arch: Option<WinArch>,
    pub ram_mb: Option<u64>,
    pub disk_mb: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WinArch {
    Win32,
    Win64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Installer {
    /// The user provides the installer (licensed software); it must match one of the hashes.
    UserSupplied {
        prompt: String,
        sha256_any: Vec<String>,
        #[serde(default)]
        file_name_hint: Option<String>,
    },
    /// Whistle downloads it from the vendor and checks the pinned hash.
    Download {
        url: String,
        sha256: String,
        #[serde(default)]
        file_name: Option<String>,
    },
    /// A package inside the runtime, e.g. apt inside the WSL distro.
    Package {
        manager: String,
        #[serde(rename = "ref")]
        reference: String,
    },
}

/// One typed action. There is no shell step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    Winetricks(Vec<String>),
    DllOverride(BTreeMap<String, String>),
    Registry(RegistryStep),
    Copy(CopyStep),
    Env(BTreeMap<String, String>),
    RunInstaller(RunInstaller),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryStep {
    pub key: String,
    pub value: BTreeMap<String, RegValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RegValue {
    Dword(u32),
    String(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CopyStep {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunInstaller {
    #[serde(default)]
    pub args: Vec<String>,
    pub timeout_s: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launcher {
    pub name: String,
    pub exe: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub mime: Vec<String>,
    #[serde(default)]
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthCheck {
    pub launcher: String,
    pub timeout_s: Option<u64>,
    pub window_title: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    Vm,
    #[default]
    None,
}

/// Variables a recipe may never set; same list as the schema.
const BLOCKED_ENV: &[&str] = &[
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "PATH",
    "HOME",
    "PYTHONPATH",
    "WINEPREFIX",
    "WINESERVER",
    "WINELOADER",
];

impl Recipe {
    pub fn from_value(spec: &serde_json::Value) -> Result<Self> {
        let recipe: Self = serde_json::from_value(spec.clone())?;
        if recipe.schema_version != SCHEMA_VERSION {
            bail!("recipe schema version {} is not supported", recipe.schema_version);
        }
        recipe.check()?;
        Ok(recipe)
    }

    /// The sandbox rules: no escaping the prefix, no hijacking the loader.
    pub fn check(&self) -> Result<()> {
        let windows = matches!(self.runtime, Runtime::Wine | Runtime::Proton | Runtime::Vm);
        for launcher in &self.launchers {
            if windows {
                check_windows_path(&launcher.exe)?;
            } else if launcher.exe.contains("..") {
                bail!("launcher {:?}: '..' is not allowed", launcher.name);
            }
        }
        for step in &self.steps {
            match step {
                Step::Copy(copy) => {
                    if copy.from.starts_with(['/', '\\']) || copy.from.contains("..") || copy.from.contains(':') {
                        bail!("copy: {:?} must be a relative path inside the installer media", copy.from);
                    }
                    check_windows_path(&copy.to)?;
                }
                Step::Env(vars) => {
                    for name in vars.keys() {
                        let valid = name.chars().next().is_some_and(|c| c.is_ascii_uppercase() || c == '_')
                            && name.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
                        if !valid || BLOCKED_ENV.contains(&name.as_str()) {
                            bail!("env: setting {name} is not allowed");
                        }
                    }
                }
                Step::Registry(reg) => {
                    reg_root(&reg.key)?;
                }
                Step::DllOverride(dlls) => {
                    for (dll, mode) in dlls {
                        let ok_mode = ["native", "builtin", "native,builtin", "builtin,native", "disabled"]
                            .contains(&mode.as_str());
                        let ok_name = dll.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
                        if !ok_mode || !ok_name {
                            bail!("dll_override: {dll}={mode} is not allowed");
                        }
                    }
                }
                Step::Winetricks(verbs) => {
                    for verb in verbs {
                        if verb.is_empty() || !verb.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_=.".contains(c)) {
                            bail!("winetricks: {verb:?} is not a valid verb");
                        }
                    }
                }
                Step::RunInstaller(_) => {}
            }
        }
        Ok(())
    }
}

/// `C:\...` with no `..` component.
pub fn check_windows_path(path: &str) -> Result<()> {
    let bytes = path.as_bytes();
    let drive = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\';
    if !drive || path.contains("..") || path.contains('\n') {
        bail!("{path:?} must be an absolute Windows path like C:\\Program Files\\App\\app.exe");
    }
    Ok(())
}

/// Expand the short hive names a recipe may use to what regedit expects.
pub fn reg_root(key: &str) -> Result<String> {
    let (hive, rest) = key.split_once('\\').unwrap_or((key, ""));
    let full = match hive {
        "HKCU" | "HKEY_CURRENT_USER" => "HKEY_CURRENT_USER",
        "HKLM" | "HKEY_LOCAL_MACHINE" => "HKEY_LOCAL_MACHINE",
        _ => bail!("registry: {key:?} must start with HKCU or HKLM"),
    };
    if rest.is_empty() || rest.contains('\n') || rest.contains(']') {
        bail!("registry: invalid key {key:?}");
    }
    Ok(format!("{full}\\{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const RECIPES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../whistle-recipes/recipes");

    #[test]
    fn parses_every_recipe_in_whistle_recipes() {
        let dir = Path::new(RECIPES);
        if !dir.is_dir() {
            eprintln!("skipping: {RECIPES} not found");
            return;
        }
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "yaml" || e == "yml") {
                let yaml: serde_json::Value = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                let recipe = Recipe::from_value(&yaml).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()));
                assert_eq!(path.file_stem().unwrap().to_str().unwrap(), recipe.id);
                count += 1;
            }
        }
        assert!(count > 0);
    }

    #[test]
    fn parses_typed_steps() {
        let recipe = Recipe::from_value(&serde_json::json!({
            "schema_version": 1, "id": "org.example.App", "app": "org.example.App", "runtime": "wine",
            "installer": {"type": "download", "url": "https://example.org/a.exe", "sha256": "0".repeat(64)},
            "steps": [
                {"winetricks": ["corefonts"]},
                {"registry": {"key": "HKCU\\Software\\X", "value": {"a": 1, "b": "s"}}},
                {"run_installer": {"args": ["/S"]}}
            ],
            "launchers": [{"name": "App", "exe": "C:\\App\\app.exe"}]
        }))
        .unwrap();
        let Step::Registry(reg) = &recipe.steps[1] else { panic!() };
        assert_eq!(reg.value["a"], RegValue::Dword(1));
        assert_eq!(reg.value["b"], RegValue::String("s".into()));
    }

    fn with_step(step: serde_json::Value) -> Result<Recipe> {
        Recipe::from_value(&serde_json::json!({
            "schema_version": 1, "id": "org.example.App", "app": "org.example.App", "runtime": "wine",
            "installer": {"type": "download", "url": "https://example.org/a.exe", "sha256": "0".repeat(64)},
            "steps": [step],
            "launchers": [{"name": "App", "exe": "C:\\App\\app.exe"}]
        }))
    }

    #[test]
    fn rejects_sandbox_escapes() {
        assert!(with_step(serde_json::json!({"env": {"LD_PRELOAD": "/x.so"}})).is_err());
        assert!(with_step(serde_json::json!({"copy": {"from": "../etc/passwd", "to": "C:\\x"}})).is_err());
        assert!(with_step(serde_json::json!({"copy": {"from": "a", "to": "C:\\..\\..\\x"}})).is_err());
        assert!(with_step(serde_json::json!({"shell": "rm -rf ~"})).is_err());
        assert!(with_step(serde_json::json!({"registry": {"key": "HKCR\\x", "value": {"a": 1}}})).is_err());
        assert!(with_step(serde_json::json!({"winetricks": ["a; rm -rf ~"]})).is_err());
    }
}
