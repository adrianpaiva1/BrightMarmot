//! Which of the open windows are candidates for "Add Game". Pure: the window list comes in as data.

use crate::config::{Config, exe_name_from_path};

#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub exe_path: String,
    pub title: String,
    pub visible: bool,
    pub has_owner: bool,
    pub tool_window: bool,
    pub cloaked: bool,
}

impl WindowInfo {
    /// Visible, unowned, not a tool window, not cloaked, and titled. Needs no exe path, so the
    /// window list can check it before looking the path up.
    pub fn is_app_window(&self) -> bool {
        self.visible && !self.has_owner && !self.tool_window && !self.cloaked && !self.title.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunningApp {
    pub exe_path: String,
    pub exe_name: String,
    pub process_name: String,
    pub title: String,
}

pub fn running_apps(windows: &[WindowInfo], self_exe_name: &str, config: &Config) -> Vec<RunningApp> {
    let mut out: Vec<RunningApp> = Vec::new();
    for w in windows {
        if !w.is_app_window() || w.exe_path.is_empty() {
            continue;
        }
        let exe_name = exe_name_from_path(&w.exe_path);
        if exe_name.eq_ignore_ascii_case(self_exe_name)
            || exe_name.eq_ignore_ascii_case("applicationframehost.exe")
            || config.find_game(&exe_name).is_some()
            || out.iter().any(|a| a.exe_name.eq_ignore_ascii_case(&exe_name))
        {
            continue;
        }
        let process_name = match exe_name.rsplit_once('.') {
            Some((stem, _)) if !stem.is_empty() => stem.to_string(),
            _ => exe_name.clone(),
        };
        out.push(RunningApp { exe_path: w.exe_path.clone(), exe_name, process_name, title: w.title.clone() });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(path: &str, title: &str) -> WindowInfo {
        WindowInfo { exe_path: path.into(), title: title.into(), visible: true, has_owner: false, tool_window: false, cloaked: false }
    }

    #[test]
    fn keeps_plain_app_windows_with_names() {
        let r = running_apps(&[w(r"C:\B\brave.exe", "South Park - Brave")], "brightmarmot.exe", &Config::default());
        assert_eq!(
            r,
            vec![RunningApp {
                exe_path: r"C:\B\brave.exe".into(),
                exe_name: "brave.exe".into(),
                process_name: "brave".into(),
                title: "South Park - Brave".into()
            }]
        );
    }

    #[test]
    fn filters_hidden_owned_tool_cloaked_untitled_unknown() {
        let mut v = vec![];
        for f in 0..6 {
            let mut x = w(&format!(r"C:\a{f}.exe"), "T");
            match f {
                0 => x.visible = false,
                1 => x.has_owner = true,
                2 => x.tool_window = true,
                3 => x.cloaked = true,
                4 => x.title = "  ".into(),
                _ => x.exe_path = String::new(),
            }
            v.push(x);
        }
        assert!(running_apps(&v, "brightmarmot.exe", &Config::default()).is_empty());
    }

    #[test]
    fn excludes_self_framehost_and_added_games_and_dedupes() {
        let mut c = Config::default();
        c.add_game(r"C:\g\cod.exe");
        let v = [
            w(r"C:\x\BrightMarmot.exe", "BrightMarmot"),
            w(r"C:\W\ApplicationFrameHost.exe", "Calc"),
            w(r"C:\g\COD.exe", "Call of Duty"),
            w(r"C:\b\brave.exe", "One"),
            w(r"C:\b\brave.exe", "Two"),
        ];
        let r = running_apps(&v, "brightmarmot.exe", &c);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].title, "One");
    }
}
