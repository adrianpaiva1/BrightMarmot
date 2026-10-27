use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Control {
    pub enabled: bool,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlKind {
    Gamma,
    Contrast,
    Vibrance,
    Hue,
    ShadowBoost,
}

impl ControlKind {
    pub const ALL: [ControlKind; 5] = [
        ControlKind::Gamma,
        ControlKind::Contrast,
        ControlKind::Vibrance,
        ControlKind::Hue,
        ControlKind::ShadowBoost,
    ];

    pub fn range(self) -> (f64, f64) {
        match self {
            ControlKind::Gamma => (0.5, 3.0),
            ControlKind::Contrast => (0.5, 2.0),
            ControlKind::Vibrance => (0.0, 100.0),
            ControlKind::Hue => (0.0, 359.0),
            ControlKind::ShadowBoost => (0.0, 100.0),
        }
    }

    pub fn neutral(self) -> f64 {
        match self {
            ControlKind::Gamma | ControlKind::Contrast => 1.0,
            ControlKind::Vibrance => 50.0,
            ControlKind::Hue | ControlKind::ShadowBoost => 0.0,
        }
    }

    pub fn decimals(self) -> usize {
        match self {
            ControlKind::Gamma | ControlKind::Contrast => 2,
            _ => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ControlKind::Gamma => "Gamma",
            ControlKind::Contrast => "Contrast",
            ControlKind::Vibrance => "Vibrance",
            ControlKind::Hue => "Hue",
            ControlKind::ShadowBoost => "Shadow Boost",
        }
    }

    fn neutral_control(self) -> Control {
        Control { enabled: false, value: self.neutral() }
    }
}

fn default_gamma() -> Control { ControlKind::Gamma.neutral_control() }
fn default_contrast() -> Control { ControlKind::Contrast.neutral_control() }
fn default_vibrance() -> Control { ControlKind::Vibrance.neutral_control() }
fn default_hue() -> Control { ControlKind::Hue.neutral_control() }
fn default_shadow_boost() -> Control { ControlKind::ShadowBoost.neutral_control() }

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Game {
    pub exe_name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_gamma")]
    pub gamma: Control,
    #[serde(default = "default_contrast")]
    pub contrast: Control,
    #[serde(default = "default_shadow_boost")]
    pub shadow_boost: Control,
    #[serde(default = "default_vibrance")]
    pub vibrance: Control,
    #[serde(default = "default_hue")]
    pub hue: Control,
}

/// File name of `path`, splitting on both `\` and `/`.
pub fn exe_name_from_path(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn display_name(exe_name: &str) -> String {
    let n = exe_name.len();
    if n >= 4 && exe_name.is_char_boundary(n - 4) && exe_name[n - 4..].eq_ignore_ascii_case(".exe") {
        exe_name[..n - 4].to_string()
    } else {
        exe_name.to_string()
    }
}

impl Game {
    pub fn new(path: &str) -> Game {
        let exe_name = exe_name_from_path(path);
        Game {
            name: display_name(&exe_name),
            exe_name,
            path: path.to_string(),
            gamma: default_gamma(),
            contrast: default_contrast(),
            shadow_boost: default_shadow_boost(),
            vibrance: default_vibrance(),
            hue: default_hue(),
        }
    }

    pub fn control(&self, k: ControlKind) -> Control {
        match k {
            ControlKind::Gamma => self.gamma,
            ControlKind::Contrast => self.contrast,
            ControlKind::Vibrance => self.vibrance,
            ControlKind::Hue => self.hue,
            ControlKind::ShadowBoost => self.shadow_boost,
        }
    }

    pub fn control_mut(&mut self, k: ControlKind) -> &mut Control {
        match k {
            ControlKind::Gamma => &mut self.gamma,
            ControlKind::Contrast => &mut self.contrast,
            ControlKind::Vibrance => &mut self.vibrance,
            ControlKind::Hue => &mut self.hue,
            ControlKind::ShadowBoost => &mut self.shadow_boost,
        }
    }

    /// True when any control that is applied through the gamma ramp is enabled.
    pub fn uses_ramp(&self) -> bool {
        self.gamma.enabled || self.contrast.enabled || self.shadow_boost.enabled
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Config {
    #[serde(default)]
    pub start_with_windows: bool,
    #[serde(default)]
    pub gamma_range_prompt_answered: bool,
    #[serde(default)]
    pub games: Vec<Game>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AddOutcome {
    Added(usize),
    AlreadyExists(usize),
}

impl Config {
    pub fn find_game(&self, exe_name: &str) -> Option<&Game> {
        self.games.iter().find(|g| g.exe_name.eq_ignore_ascii_case(exe_name))
    }

    pub fn add_game(&mut self, path: &str) -> AddOutcome {
        let exe_name = exe_name_from_path(path);
        if let Some(i) = self.games.iter().position(|g| g.exe_name.eq_ignore_ascii_case(&exe_name)) {
            return AddOutcome::AlreadyExists(i);
        }
        self.games.push(Game::new(path));
        AddOutcome::Added(self.games.len() - 1)
    }

    pub fn remove_game(&mut self, index: usize) {
        if index < self.games.len() {
            self.games.remove(index);
        }
    }

    pub fn from_json(s: &str) -> Result<Config, serde_json::Error> {
        let mut c: Config = serde_json::from_str(s)?;
        c.sanitize();
        Ok(c)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("config serializes to JSON")
    }

    fn sanitize(&mut self) {
        let mut seen: Vec<String> = Vec::new();
        self.games.retain(|g| {
            if g.exe_name.is_empty() {
                return false;
            }
            let key = g.exe_name.to_ascii_lowercase();
            if seen.contains(&key) {
                return false;
            }
            seen.push(key);
            true
        });
        for g in &mut self.games {
            if g.name.is_empty() {
                g.name = display_name(&g.exe_name);
            }
            for k in ControlKind::ALL {
                let (lo, hi) = k.range();
                let c = g.control_mut(k);
                c.value = c.value.clamp(lo, hi);
            }
        }
    }
}

pub enum LoadOutcome {
    Loaded,
    Missing,
    Corrupt,
}

pub fn load(path: &Path) -> (Config, LoadOutcome) {
    if !path.exists() {
        return (Config::default(), LoadOutcome::Missing);
    }
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| Config::from_json(&s).ok());
    match parsed {
        Some(c) => (c, LoadOutcome::Loaded),
        None => {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let _ = std::fs::rename(path, PathBuf::from(bad));
            (Config::default(), LoadOutcome::Corrupt)
        }
    }
}

pub fn save(path: &Path, config: &Config) -> std::io::Result<()> {
    write_atomic(path, &config.to_json())
}

/// `%APPDATA%\BrightMarmot\config.json`.
pub fn default_path() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("BrightMarmot").join("config.json"))
}

/// Writes `<name>.tmp` next to `path` (creating parent dirs), then renames it over `path`.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, contents)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn exe_name_from_windows_and_unix_paths() {
        assert_eq!(exe_name_from_path(r"C:\XboxGames\Call of Duty\Content\cod.exe"), "cod.exe");
        assert_eq!(exe_name_from_path("D:/Games/bf6.exe"), "bf6.exe");
        assert_eq!(exe_name_from_path("bf6.exe"), "bf6.exe");
    }
    #[test] fn new_game_defaults() {
        let g = Game::new(r"C:\Games\COD.EXE");
        assert_eq!((g.exe_name.as_str(), g.name.as_str(), g.path.as_str()), ("COD.EXE", "COD", r"C:\Games\COD.EXE"));
        for k in ControlKind::ALL { assert!(!g.control(k).enabled); assert_eq!(g.control(k).value, k.neutral()); }
        assert_eq!((g.gamma.value, g.contrast.value, g.vibrance.value, g.hue.value, g.shadow_boost.value), (1.0, 1.0, 50.0, 0.0, 0.0));
    }
    #[test] fn uses_ramp_only_for_gamma_contrast_shadow() {
        let mut g = Game::new("a.exe"); g.vibrance.enabled = true; g.hue.enabled = true;
        assert!(!g.uses_ramp());
        g.shadow_boost.enabled = true; assert!(g.uses_ramp());
    }
    #[test] fn find_game_is_case_insensitive() {
        let mut c = Config::default(); c.add_game(r"C:\x\cod.exe");
        assert!(c.find_game("COD.EXE").is_some());
        assert!(c.find_game("bf6.exe").is_none());
    }
    #[test] fn add_duplicate_returns_existing() {
        let mut c = Config::default();
        assert_eq!(c.add_game(r"C:\a\cod.exe"), AddOutcome::Added(0));
        assert_eq!(c.add_game(r"D:\b\COD.exe"), AddOutcome::AlreadyExists(0));
        assert_eq!(c.games.len(), 1);
    }
    #[test] fn json_roundtrip() {
        let mut c = Config::default(); c.start_with_windows = true; c.add_game(r"C:\a\cod.exe");
        c.games[0].gamma = Control { enabled: true, value: 1.3 };
        assert_eq!(Config::from_json(&c.to_json()).unwrap(), c);
    }
    #[test] fn missing_fields_get_neutral_defaults() {
        let c = Config::from_json(r#"{"games":[{"exe_name":"cod.exe"}]}"#).unwrap();
        let g = &c.games[0];
        assert!(!c.start_with_windows && !c.gamma_range_prompt_answered);
        assert_eq!((g.name.as_str(), g.path.as_str()), ("cod", ""));
        assert_eq!((g.gamma.value, g.vibrance.value, g.gamma.enabled), (1.0, 50.0, false));
    }
    #[test] fn out_of_range_values_are_clamped() {
        let c = Config::from_json(r#"{"games":[{"exe_name":"a.exe",
            "gamma":{"enabled":true,"value":99},"contrast":{"enabled":true,"value":0.1},
            "hue":{"enabled":true,"value":400},"vibrance":{"enabled":true,"value":-5}}]}"#).unwrap();
        let g = &c.games[0];
        assert_eq!((g.gamma.value, g.contrast.value, g.hue.value, g.vibrance.value), (3.0, 0.5, 359.0, 0.0));
    }
    #[test] fn case_duplicates_and_empty_exe_dropped() {
        let c = Config::from_json(r#"{"games":[{"exe_name":"cod.exe","path":"first"},
            {"exe_name":"COD.EXE","path":"second"},{"exe_name":""}]}"#).unwrap();
        assert_eq!(c.games.len(), 1);
        assert_eq!(c.games[0].path, "first");
    }
    #[test] fn load_missing_file() {
        let d = tempfile::tempdir().unwrap();
        let (c, o) = load(&d.path().join("config.json"));
        assert_eq!(c, Config::default()); assert!(matches!(o, LoadOutcome::Missing));
    }
    #[test] fn load_corrupt_renames_aside() {
        let d = tempfile::tempdir().unwrap(); let p = d.path().join("config.json");
        std::fs::write(&p, "not json").unwrap();
        let (c, o) = load(&p);
        assert_eq!(c, Config::default()); assert!(matches!(o, LoadOutcome::Corrupt));
        assert!(!p.exists());
        assert_eq!(std::fs::read_to_string(d.path().join("config.json.bad")).unwrap(), "not json");
    }
    #[test] fn save_creates_dirs_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap(); let p = d.path().join("sub").join("config.json");
        let mut c = Config::default(); c.add_game("bf6.exe");
        save(&p, &c).unwrap();
        assert_eq!(load(&p).0, c);
        assert!(!d.path().join("sub").join("config.json.tmp").exists());
    }
}
