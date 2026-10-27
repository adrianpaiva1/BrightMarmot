//! Apply/restore state machine. Pure: all display access goes through `DisplayBackend`.

use std::collections::BTreeMap;

use crate::config::{Config, Game};
use crate::ramp::{layered, CurveParams, GammaRamp};
use crate::restore::{RestoreFile, SavedNormal};
use crate::vibrance::{ui_to_raw, VibranceInfo};

/// The window in front: its exe name and the GDI device name of its monitor (e.g. `\\.\DISPLAY1`).
#[derive(Clone, Debug, PartialEq)]
pub struct Foreground {
    pub exe_name: String,
    pub monitor: String,
}

pub trait DisplayBackend {
    fn get_ramp(&mut self, monitor: &str) -> Option<GammaRamp>;
    fn set_ramp(&mut self, monitor: &str, ramp: &GammaRamp) -> bool;
    fn get_vibrance(&mut self, monitor: &str) -> Option<VibranceInfo>;
    fn set_vibrance(&mut self, monitor: &str, raw: i32) -> bool;
    fn get_hue(&mut self, monitor: &str) -> Option<u32>;
    fn set_hue(&mut self, monitor: &str, degrees: u32) -> bool;
}

/// A monitor we have captured the normal colours of, and which parts we have changed.
struct MonitorState {
    normal: SavedNormal,
    info: Option<VibranceInfo>,
    ramp_modified: bool,
    vibrance_modified: bool,
    hue_modified: bool,
}

impl MonitorState {
    fn fully_restored(&self) -> bool {
        !(self.ramp_modified || self.vibrance_modified || self.hue_modified)
    }
}

pub struct Engine {
    active: BTreeMap<String, MonitorState>,
}

impl Engine {
    pub fn new() -> Engine {
        Engine { active: BTreeMap::new() }
    }

    /// Applies the matching game's colours to the foreground monitor and restores every other
    /// active monitor. Returns true when a game matched.
    pub fn update(
        &mut self,
        fg: Option<&Foreground>,
        config: &Config,
        backend: &mut dyn DisplayBackend,
        store: &mut RestoreFile,
    ) -> bool {
        let target = fg.and_then(|f| config.find_game(&f.exe_name).map(|g| (f.monitor.as_str(), g)));

        let others: Vec<String> = self
            .active
            .keys()
            .filter(|m| target.is_none_or(|(t, _)| t != m.as_str()))
            .cloned()
            .collect();
        for monitor in others {
            self.restore_monitor(&monitor, backend, store);
        }

        let Some((monitor, game)) = target else { return false };
        if !self.active.contains_key(monitor) {
            let normal = SavedNormal {
                ramp: backend.get_ramp(monitor),
                vibrance: None,
                hue: backend.get_hue(monitor),
            };
            let info = backend.get_vibrance(monitor);
            let normal = SavedNormal { vibrance: info.map(|i| i.current), ..normal };
            store.insert(monitor, normal.clone());
            let _ = store.save();
            self.active.insert(
                monitor.to_string(),
                MonitorState { normal, info, ramp_modified: false, vibrance_modified: false, hue_modified: false },
            );
        }
        if let Some(state) = self.active.get_mut(monitor) {
            sync(monitor, state, Some(game), backend);
        }
        true
    }

    pub fn restore_all(&mut self, backend: &mut dyn DisplayBackend, store: &mut RestoreFile) {
        let all: Vec<String> = self.active.keys().cloned().collect();
        for monitor in all {
            self.restore_monitor(&monitor, backend, store);
        }
    }

    #[cfg(test)]
    pub fn active_monitors(&self) -> Vec<String> {
        self.active.keys().cloned().collect()
    }

    /// On full success the monitor leaves `active` and the store; otherwise it stays for a retry.
    fn restore_monitor(&mut self, monitor: &str, backend: &mut dyn DisplayBackend, store: &mut RestoreFile) {
        let Some(state) = self.active.get_mut(monitor) else { return };
        sync(monitor, state, None, backend);
        if state.fully_restored() {
            self.active.remove(monitor);
            store.remove(monitor);
            let _ = store.save();
        }
    }
}

/// Brings each part to what `game` wants (`None` = normal). Parts whose normal value is unknown
/// are never touched; a rejected set leaves the part's modified flag as it was.
fn sync(monitor: &str, st: &mut MonitorState, game: Option<&Game>, backend: &mut dyn DisplayBackend) {
    let want_ramp = game
        .filter(|g| g.uses_ramp())
        .zip(st.normal.ramp.as_ref())
        .map(|(g, normal)| layered(normal, &CurveParams::from_game(g)));
    if let Some(ramp) = want_ramp {
        if backend.set_ramp(monitor, &ramp) {
            st.ramp_modified = true;
        }
    } else if st.ramp_modified {
        if let Some(normal) = &st.normal.ramp {
            if backend.set_ramp(monitor, normal) {
                st.ramp_modified = false;
            }
        }
    }

    let want_vibrance = game
        .filter(|g| g.vibrance.enabled)
        .zip(st.info.as_ref())
        .map(|(g, info)| ui_to_raw(g.vibrance.value, info));
    if let Some(raw) = want_vibrance {
        if backend.set_vibrance(monitor, raw) {
            st.vibrance_modified = true;
        }
    } else if st.vibrance_modified {
        if let Some(normal) = st.normal.vibrance {
            if backend.set_vibrance(monitor, normal) {
                st.vibrance_modified = false;
            }
        }
    }

    let want_hue = game
        .filter(|g| g.hue.enabled && st.normal.hue.is_some())
        .map(|g| g.hue.value.round().clamp(0.0, 359.0) as u32);
    if let Some(degrees) = want_hue {
        if backend.set_hue(monitor, degrees) {
            st.hue_modified = true;
        }
    } else if st.hue_modified {
        if let Some(normal) = st.normal.hue {
            if backend.set_hue(monitor, normal) {
                st.hue_modified = false;
            }
        }
    }
}

/// Undoes a crashed run: sets every saved part back, then forgets the store regardless of failures.
pub fn recover(store: &mut RestoreFile, backend: &mut dyn DisplayBackend) {
    for (monitor, saved) in store.entries() {
        if let Some(ramp) = &saved.ramp {
            backend.set_ramp(monitor, ramp);
        }
        if let Some(raw) = saved.vibrance {
            backend.set_vibrance(monitor, raw);
        }
        if let Some(degrees) = saved.hue {
            backend.set_hue(monitor, degrees);
        }
    }
    store.clear();
    let _ = store.save();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Control, Game};
    use crate::ramp::{layered, CurveParams, GammaRamp};
    use crate::restore::{RestoreFile, SavedNormal};
    use crate::vibrance::VibranceInfo;
    use std::collections::BTreeMap;

    const D1: &str = r"\\.\DISPLAY1";
    const D2: &str = r"\\.\DISPLAY2";

    #[derive(Default)]
    struct Mon {
        ramp: Option<GammaRamp>,
        vib: Option<VibranceInfo>,
        hue: Option<u32>,
        reject_ramp: bool,
        reject_all: bool,
        ramp_gets: usize,
        ramp_sets: usize,
        hue_sets: usize,
    }
    #[derive(Default)]
    struct Fake {
        m: BTreeMap<String, Mon>,
    }
    impl DisplayBackend for Fake {
        fn get_ramp(&mut self, monitor: &str) -> Option<GammaRamp> {
            let m = self.m.get_mut(monitor)?;
            m.ramp_gets += 1;
            m.ramp
        }
        fn set_ramp(&mut self, monitor: &str, ramp: &GammaRamp) -> bool {
            let Some(m) = self.m.get_mut(monitor) else { return false };
            m.ramp_sets += 1;
            if m.reject_all || m.reject_ramp {
                return false;
            }
            m.ramp = Some(*ramp);
            true
        }
        fn get_vibrance(&mut self, monitor: &str) -> Option<VibranceInfo> {
            self.m.get(monitor)?.vib
        }
        fn set_vibrance(&mut self, monitor: &str, raw: i32) -> bool {
            let Some(m) = self.m.get_mut(monitor) else { return false };
            if m.reject_all {
                return false;
            }
            match m.vib.as_mut() {
                Some(v) => {
                    v.current = raw;
                    true
                }
                None => false,
            }
        }
        fn get_hue(&mut self, monitor: &str) -> Option<u32> {
            self.m.get(monitor)?.hue
        }
        fn set_hue(&mut self, monitor: &str, degrees: u32) -> bool {
            let Some(m) = self.m.get_mut(monitor) else { return false };
            m.hue_sets += 1;
            if m.reject_all {
                return false;
            }
            m.hue = Some(degrees);
            true
        }
    }

    fn normal() -> GammaRamp {
        let mut r = crate::ramp::identity();
        r[1] = [20000; 256];
        r
    }
    fn info() -> VibranceInfo {
        VibranceInfo { current: 5, min: 0, max: 63, default: 0 }
    }
    fn fake(mons: &[&str]) -> Fake {
        let mut f = Fake::default();
        for name in mons {
            f.m.insert(
                name.to_string(),
                Mon { ramp: Some(normal()), vib: Some(info()), hue: Some(0), ..Mon::default() },
            );
        }
        f
    }
    fn game(exe: &str, gamma: Option<f64>, vib: Option<f64>, hue: Option<f64>) -> Game {
        let mut g = Game::new(exe);
        if let Some(v) = gamma {
            g.gamma = Control { enabled: true, value: v };
        }
        if let Some(v) = vib {
            g.vibrance = Control { enabled: true, value: v };
        }
        if let Some(v) = hue {
            g.hue = Control { enabled: true, value: v };
        }
        g
    }
    fn fg(exe: &str, mon: &str) -> Foreground {
        Foreground { exe_name: exe.into(), monitor: mon.into() }
    }
    fn cfg(games: Vec<Game>) -> Config {
        Config { games, ..Config::default() }
    }

    #[test]
    fn game_in_front_applies_switched_on_controls() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), Some(70.0), None)]);
        assert!(e.update(Some(&fg("COD.EXE", D1)), &c, &mut f, &mut s));
        let m = &f.m[D1];
        assert_eq!(m.ramp, Some(layered(&normal(), &CurveParams { gamma: 2.0, contrast: 1.0, shadow_boost: 0.0 })));
        assert_eq!(m.vib.unwrap().current, 25);
        assert_eq!(m.hue_sets, 0);
    }

    #[test]
    fn normal_saved_to_store_on_first_apply() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), Some(70.0), None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        assert_eq!(
            s.entries()[D1],
            SavedNormal { ramp: Some(normal()), vibrance: Some(5), hue: Some(0) }
        );
    }

    #[test]
    fn non_game_in_front_restores_normal() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), Some(70.0), None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        assert!(!e.update(None, &c, &mut f, &mut s));
        let m = &f.m[D1];
        assert_eq!(m.ramp, Some(normal()));
        assert_eq!(m.vib.unwrap().current, 5);
        assert!(e.active_monitors().is_empty());
        assert!(s.entries().is_empty());
    }

    #[test]
    fn ramp_untouched_when_all_ramp_switches_off() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", None, Some(70.0), None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        e.update(None, &c, &mut f, &mut s);
        assert_eq!(f.m[D1].ramp_sets, 0);
    }

    #[test]
    fn repeat_update_reapplies_without_recapture() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), None, None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        assert_eq!(f.m[D1].ramp_sets, 2);
        assert_eq!(f.m[D1].ramp_gets, 1);
    }

    #[test]
    fn switch_game_same_monitor_resets_unused_parts() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), None, None), game("bf6.exe", None, Some(70.0), None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        e.update(Some(&fg("bf6.exe", D1)), &c, &mut f, &mut s);
        let m = &f.m[D1];
        assert_eq!(m.ramp, Some(normal()));
        assert_eq!(m.vib.unwrap().current, 25);
        assert_eq!(m.ramp_gets, 1);
        assert_eq!(s.entries()[D1].ramp, Some(normal()));
    }

    #[test]
    fn switch_to_game_on_other_monitor_restores_first() {
        let (mut f, mut s, mut e) = (fake(&[D1, D2]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), None, None), game("bf6.exe", Some(2.0), None, None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        e.update(Some(&fg("bf6.exe", D2)), &c, &mut f, &mut s);
        assert_eq!(f.m[D1].ramp, Some(normal()));
        assert_ne!(f.m[D2].ramp, Some(normal()));
        assert_eq!(e.active_monitors(), vec![D2.to_string()]);
    }

    #[test]
    fn rejected_ramp_still_applies_vibrance() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        f.m.get_mut(D1).unwrap().reject_ramp = true;
        let c = cfg(vec![game("cod.exe", Some(2.0), Some(70.0), None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        assert_eq!(f.m[D1].vib.unwrap().current, 25);
        assert_eq!(f.m[D1].ramp, Some(normal()));
        e.update(None, &c, &mut f, &mut s);
        assert_eq!(f.m[D1].ramp_sets, 1);
    }

    #[test]
    fn failed_restore_is_retried_next_update() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), None, None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        f.m.get_mut(D1).unwrap().reject_all = true;
        e.update(None, &c, &mut f, &mut s);
        assert_eq!(e.active_monitors(), vec![D1.to_string()]);
        assert!(s.entries().contains_key(D1));
        f.m.get_mut(D1).unwrap().reject_all = false;
        e.update(None, &c, &mut f, &mut s);
        assert_eq!(f.m[D1].ramp, Some(normal()));
        assert!(e.active_monitors().is_empty());
        assert!(s.entries().is_empty());
    }

    #[test]
    fn missing_normal_ramp_leaves_ramp_alone() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        f.m.get_mut(D1).unwrap().ramp = None;
        let c = cfg(vec![game("cod.exe", Some(2.0), None, None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        assert_eq!(f.m[D1].ramp_sets, 0);
    }

    #[test]
    fn restore_all_restores_every_active_monitor() {
        let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
        let c = cfg(vec![game("cod.exe", Some(2.0), Some(70.0), None)]);
        e.update(Some(&fg("cod.exe", D1)), &c, &mut f, &mut s);
        e.restore_all(&mut f, &mut s);
        assert_eq!(f.m[D1].ramp, Some(normal()));
        assert_eq!(f.m[D1].vib.unwrap().current, 5);
        assert!(e.active_monitors().is_empty());
    }

    #[test]
    fn recover_applies_saved_normals_and_clears_store() {
        let (mut f, mut s) = (fake(&[D1]), RestoreFile::in_memory());
        s.insert(D1, SavedNormal { ramp: Some(crate::ramp::identity()), vibrance: Some(9), hue: Some(3) });
        recover(&mut s, &mut f);
        let m = &f.m[D1];
        assert_eq!(m.ramp, Some(crate::ramp::identity()));
        assert_eq!(m.vib.unwrap().current, 9);
        assert_eq!(m.hue, Some(3));
        assert!(s.entries().is_empty());
    }
}
