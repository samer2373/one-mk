use one_mk_proto::remap::{Output, RuleSpec, What};
use std::collections::BTreeMap;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const KEYBOARD_PAGE: u64 = 0x0007_0000_0000;

pub(crate) fn hid_key_to_evdev(usage: u32) -> Option<u32> {
    #[rustfmt::skip]
    const KEYS: [u8; 0x66] = [
        0, 0, 0, 0, 30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38,
        50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45, 21, 44, 2, 3,
        4, 5, 6, 7, 8, 9, 10, 11, 28, 1, 14, 15, 57, 12, 13, 26,
        27, 43, 43, 39, 40, 41, 51, 52, 53, 58, 59, 60, 61, 62, 63, 64,
        65, 66, 67, 68, 87, 88, 99, 70, 119, 110, 102, 104, 111, 107, 109, 106,
        105, 108, 103, 69, 98, 55, 74, 78, 96, 79, 80, 81, 75, 76, 77, 71,
        72, 73, 82, 83, 86, 127,
    ];
    let code = match usage {
        0x00..=0x65 => KEYS[usage as usize],
        0xe0..=0xe7 => [29, 42, 56, 125, 97, 54, 100, 126][(usage - 0xe0) as usize],
        _ => 0,
    };
    (code != 0).then_some(u32::from(code))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Mapping {
    pub source: u64,
    pub destination: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub base: Vec<Mapping>,
    pub devices: Vec<(crate::Fingerprint, Vec<Mapping>)>,
}

impl Plan {
    pub fn for_keyboard(&self, keyboard: &crate::Fingerprint) -> &[Mapping] {
        self.devices.iter().find(|(fingerprint, _)| fingerprint.matches(keyboard))
            .map_or(&self.base, |(_, mappings)| mappings)
    }
}

pub fn partition(specs: &[RuleSpec]) -> (Vec<Mapping>, Vec<RuleSpec>) {
    let mut simple = BTreeMap::new();
    for spec in specs {
        if let Ok(mapping) = compile(std::slice::from_ref(spec)) {
            simple.entry(spec.from.key).or_insert(mapping[0]);
        }
    }
    for spec in specs {
        if compile(std::slice::from_ref(spec)).is_err() { simple.remove(&spec.from.key); }
    }
    loop {
        let complex: Vec<_> = specs.iter().filter(|spec| !simple.contains_key(&spec.from.key)).collect();
        let before = simple.len();
        simple.retain(|_, mapping| !complex.iter().any(|spec| usage(spec.from.key).ok() == Some(mapping.destination)));
        if simple.len() == before { break; }
    }
    let tap = specs.iter().filter(|spec| !simple.contains_key(&spec.from.key)).cloned().collect();
    (simple.into_values().collect(), tap)
}

pub const JOURNAL: &str = "local-key-mapping.json";

pub trait Runner {
    fn keyboards(&mut self) -> Result<Vec<(u64, crate::Fingerprint)>>;
    fn read(&mut self, registry_id: u64) -> Result<Option<Vec<Mapping>>>;
    fn write(&mut self, registry_id: u64, expected: &[Mapping], mappings: &[Mapping]) -> Result<()>;
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u8,
    entries: Vec<Owned>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Owned {
    registry_id: u64,
    installed: Vec<Mapping>,
}

pub struct Lifecycle<R: Runner> {
    runner: R,
    path: PathBuf,
    journal: Journal,
}

impl<R: Runner> Lifecycle<R> {
    pub fn owned_mappings(&self) -> Vec<Mapping> {
        self.journal.entries.iter().flat_map(|entry| entry.installed.iter().copied()).collect()
    }

    pub fn recover(runner: R, dir: &Path) -> Result<Self> {
        let path = dir.join(JOURNAL);
        let journal = match std::fs::read(&path) {
            Ok(bytes) => {
                let journal: Journal = serde_json::from_slice(&bytes).context("Read local keyboard recovery record")?;
                if journal.version != 1 { bail!("Unknown local keyboard recovery record version"); }
                journal
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Journal { version: 1, entries: Vec::new() },
            Err(error) => return Err(error.into()),
        };
        let mut lifecycle = Self { runner, path, journal };
        lifecycle.restore()?;
        Ok(lifecycle)
    }

    fn save(&self) -> Result<()> {
        let dir = self.path.parent().context("Missing keyboard recovery folder")?;
        std::fs::create_dir_all(dir)?;
        let temporary = self.path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        { use std::os::unix::fs::OpenOptionsExt; options.mode(0o600); }
        let mut file = options.open(&temporary)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec(&self.journal)?)?;
        file.sync_all()?;
        std::fs::rename(temporary, &self.path)?;
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    }

    pub fn apply(&mut self, plan: &Plan) -> Result<()> {
        self.restore()?;
        let result = self.install(plan);
        if let Err(error) = result {
            return match self.restore() {
                Ok(()) => Err(error),
                Err(restore) => Err(error.context(format!("Keyboard restore also failed: {restore:#}"))),
            };
        }
        Ok(())
    }

    fn install(&mut self, plan: &Plan) -> Result<()> {
        if plan.base.is_empty() && plan.devices.iter().all(|(_, mappings)| mappings.is_empty()) { return Ok(()); }
        let keyboards = self.runner.keyboards()?;
        if keyboards.is_empty() && !plan.base.is_empty() { bail!("macOS did not report a keyboard mapping service"); }
        for (registry_id, keyboard) in keyboards {
            let desired = plan.for_keyboard(&keyboard);
            if desired.is_empty() { continue; }
            let Some(mut current) = self.runner.read(registry_id)? else { continue };
            let previous = current.clone();
            let mut installed = Vec::new();
            for mapping in desired {
                if current.contains(mapping) { continue; }
                if current.iter().any(|foreign| foreign.source == mapping.source) {
                    bail!("Another keyboard tool already changes this key; its mapping was left unchanged");
                }
                current.push(*mapping);
                installed.push(*mapping);
            }
            if installed.is_empty() { continue; }
            self.journal.entries.push(Owned { registry_id, installed });
            self.save()?;
            self.runner.write(registry_id, &previous, &current)?;
            if self.runner.read(registry_id)?.as_ref() != Some(&current) {
                bail!("macOS did not confirm the keyboard mapping");
            }
        }
        Ok(())
    }

    pub fn restore(&mut self) -> Result<()> {
        while let Some(owned) = self.journal.entries.last() {
            if let Some(mut current) = self.runner.read(owned.registry_id)? {
                let original = current.clone();
                for mapping in &owned.installed {
                    if let Some(index) = current.iter().position(|entry| entry == mapping) { current.remove(index); }
                }
                if current != original {
                    self.runner.write(owned.registry_id, &original, &current)?;
                    if self.runner.read(owned.registry_id)?.as_ref() != Some(&current) {
                        bail!("macOS did not confirm restoring the keyboard mapping");
                    }
                }
            }
            self.journal.entries.pop();
            if !self.journal.entries.is_empty() { self.save()?; }
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => {
                #[cfg(unix)]
                std::fs::File::open(self.path.parent().context("Missing keyboard recovery folder")?)?.sync_all()?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsupported {
    ConditionalRule,
    ComplexOutput,
    UnknownKey(u16),
    AmbiguousKey(u16),
}

fn usage(code: u16) -> Result<u64, Unsupported> {
    let mut usages = (4..=0x65).chain(0xe0..=0xe7)
        .filter(|&usage| hid_key_to_evdev(usage) == Some(u32::from(code)));
    let first = usages.next().ok_or(Unsupported::UnknownKey(code))?;
    if usages.next().is_some() { return Err(Unsupported::AmbiguousKey(code)); }
    Ok(KEYBOARD_PAGE | u64::from(first))
}

pub fn key_usage(code: u16) -> Option<u64> { usage(code).ok() }

pub fn restored_keys(mappings: &[Mapping], observed: &[u32], physical: &[u32]) -> Vec<u32> {
    let affected: Vec<_> = mappings.iter().flat_map(|mapping| [mapping.source, mapping.destination])
        .filter_map(|usage| hid_key_to_evdev((usage & 0xffff) as u32)).collect();
    let mut keys: Vec<_> = observed.iter().copied().filter(|code| !affected.contains(code)).collect();
    keys.extend(physical.iter().copied().filter(|code| affected.contains(code)));
    keys.sort_unstable();
    keys.dedup();
    keys
}

pub fn emergency_keys(physical: &[u32]) -> bool {
    physical.contains(&1) && [29, 97].iter().any(|code| physical.contains(code))
        && [56, 100].iter().any(|code| physical.contains(code))
        && [42, 54].iter().any(|code| physical.contains(code))
}

pub fn compile(specs: &[RuleSpec]) -> Result<Vec<Mapping>, Unsupported> {
    let mut mappings = BTreeMap::new();
    for spec in specs {
        if spec.layer.is_some() || spec.from.has_mods() { return Err(Unsupported::ConditionalRule); }
        let What::To(Output::Chord(output)) = &spec.what else { return Err(Unsupported::ComplexOutput) };
        if output.mods != 0 { return Err(Unsupported::ComplexOutput); }
        let source = usage(spec.from.key)?;
        let destination = usage(output.key)?;
        mappings.entry(source).or_insert(destination);
    }
    Ok(mappings.into_iter().map(|(source, destination)| Mapping { source, destination }).collect())
}

pub fn property(mappings: &[Mapping]) -> String {
    let entries: Vec<_> = mappings.iter().map(|mapping| format!(
        "{{\"HIDKeyboardModifierMappingSrc\":{},\"HIDKeyboardModifierMappingDst\":{}}}",
        mapping.source, mapping.destination,
    )).collect();
    format!("{{\"UserKeyMapping\":[{}]}}", entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_ID: AtomicU64 = AtomicU64::new(0);

    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("one-mk-hidmap-{}-{}", std::process::id(), TEST_ID.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[derive(Default)]
    struct FakeState {
        maps: BTreeMap<u64, Vec<Mapping>>,
        writes: usize,
        scans: usize,
        crash: bool,
        refuse: bool,
        concurrent: Option<Vec<Mapping>>,
    }

    #[derive(Clone)]
    struct FakeHidutil {
        state: Rc<RefCell<FakeState>>,
        journal: PathBuf,
    }

    impl FakeHidutil {
        fn new(folder: &Folder) -> Self {
            Self { state: Rc::new(RefCell::new(FakeState { maps: BTreeMap::from([(1, Vec::new()), (2, Vec::new())]), ..Default::default() })), journal: folder.0.join(JOURNAL) }
        }
    }

    fn keyboard(id: u64) -> crate::Fingerprint {
        crate::Fingerprint::new(crate::Bus::Usb, 1, id as u16, None, &format!("Keyboard {id}"))
    }

    impl Runner for FakeHidutil {
        fn keyboards(&mut self) -> Result<Vec<(u64, crate::Fingerprint)>> {
            self.state.borrow_mut().scans += 1;
            Ok(self.state.borrow().maps.keys().map(|&id| (id, keyboard(id))).collect())
        }
        fn read(&mut self, registry_id: u64) -> Result<Option<Vec<Mapping>>> { Ok(self.state.borrow().maps.get(&registry_id).cloned()) }
        fn write(&mut self, registry_id: u64, expected: &[Mapping], mappings: &[Mapping]) -> Result<()> {
            assert!(self.journal.exists(), "native changes must follow a durable write-ahead record");
            let mut state = self.state.borrow_mut();
            if let Some(concurrent) = state.concurrent.take() { state.maps.insert(registry_id, concurrent); }
            if state.maps.get(&registry_id).map(Vec::as_slice) != Some(expected) { bail!("fake concurrent keyboard mapping change"); }
            if state.refuse { bail!("fake macOS write failure"); }
            state.maps.insert(registry_id, mappings.to_vec());
            state.writes += 1;
            assert!(!std::mem::take(&mut state.crash), "simulated process crash after native write");
            Ok(())
        }
    }

    fn plan() -> Plan { Plan { base: compile(&[rule("capslock", "esc")]).unwrap(), ..Default::default() } }

    #[test]
    fn local_hidutil_restore_uses_physical_sources_not_stale_native_outputs_for_crossing() {
        let mappings = compile(&[rule("capslock", "ctrl"), rule("a", "b")]).unwrap();
        assert_eq!(restored_keys(&mappings, &[29, 48, 42], &[58, 30, 42]), [30, 42, 58]);
        assert_eq!(restored_keys(&mappings, &[29, 42], &[29, 42]), [29, 42]);
        assert_eq!(restored_keys(&[], &[29, 42], &[]), [29, 42]);
    }

    #[test]
    fn local_hidutil_emergency_uses_original_keys_even_when_all_four_are_remapped() {
        assert!(emergency_keys(&[1, 29, 56, 42]));
        assert!(emergency_keys(&[1, 97, 100, 54]));
        for missing in [1, 29, 56, 42] {
            assert!(!emergency_keys(&[1, 29, 56, 42].into_iter().filter(|code| *code != missing).collect::<Vec<_>>()));
        }
    }

    #[test]
    fn local_hidutil_unknown_recovery_records_never_write_native_properties() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        for bytes in [b"invalid".as_slice(), b"{\"version\":2,\"entries\":[]}".as_slice()] {
            std::fs::write(&fake.journal, bytes).unwrap();
            assert!(Lifecycle::recover(fake.clone(), &folder.0).is_err());
            assert_eq!(fake.state.borrow().writes, 0);
            assert!(fake.journal.exists());
        }
    }

    #[test]
    fn empty_local_plan_does_not_scan_keyboards_or_create_a_journal() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        lifecycle.apply(&Plan::default()).unwrap();
        lifecycle.restore().unwrap();
        assert_eq!(fake.state.borrow().scans, 0);
        assert_eq!(fake.state.borrow().writes, 0);
        assert!(!fake.journal.exists());
    }

    #[test]
    fn local_hidutil_lifecycle_apply_restore_and_rule_replacement_preserve_foreign_entries() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let foreign = compile(&[rule("a", "b")]).unwrap();
        fake.state.borrow_mut().maps.insert(1, foreign.clone());
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        lifecycle.apply(&plan()).unwrap();
        assert_eq!(fake.state.borrow().maps[&1], [foreign[0], plan().base[0]]);
        let replacement = Plan { base: compile(&[rule("capslock", "ctrl")]).unwrap(), ..Default::default() };
        lifecycle.apply(&replacement).unwrap();
        assert_eq!(fake.state.borrow().maps[&1], [foreign[0], replacement.base[0]]);
        lifecycle.restore().unwrap();
        lifecycle.restore().unwrap();
        assert_eq!(fake.state.borrow().maps[&1], foreign);
        assert!(fake.state.borrow().maps[&2].is_empty());
        assert!(!fake.journal.exists());
    }

    #[test]
    fn local_hidutil_crash_recovery_runs_before_new_rules_and_is_idempotent() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        fake.state.borrow_mut().crash = true;
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lifecycle.apply(&plan()))).is_err());
        assert!(fake.journal.exists());
        assert_eq!(fake.state.borrow().maps[&1], plan().base);
        drop(lifecycle);
        let mut recovered = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        assert!(fake.state.borrow().maps.values().all(Vec::is_empty));
        assert!(!fake.journal.exists());
        recovered.restore().unwrap();
    }

    #[test]
    fn local_hidutil_write_ahead_crash_before_native_apply_keeps_all_foreign_entries() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let foreign = compile(&[rule("a", "b")]).unwrap();
        fake.state.borrow_mut().maps.insert(1, foreign.clone());
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        lifecycle.journal.entries.push(Owned { registry_id: 1, installed: plan().base });
        lifecycle.save().unwrap();
        drop(lifecycle);
        Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        assert_eq!(fake.state.borrow().maps[&1], foreign);
        assert_eq!(fake.state.borrow().writes, 0);
    }

    #[test]
    fn local_hidutil_foreign_changes_after_apply_and_preexisting_identical_entries_are_not_owned() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        fake.state.borrow_mut().maps.insert(2, plan().base);
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        lifecycle.apply(&plan()).unwrap();
        let changed = compile(&[rule("capslock", "ctrl"), rule("a", "b")]).unwrap();
        fake.state.borrow_mut().maps.insert(1, changed.clone());
        lifecycle.restore().unwrap();
        assert_eq!(fake.state.borrow().maps[&1], changed);
        assert_eq!(fake.state.borrow().maps[&2], plan().base);
    }

    #[test]
    fn local_hidutil_conflicting_foreign_sources_are_refused_and_partial_application_is_rolled_back() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let foreign = compile(&[rule("capslock", "ctrl")]).unwrap();
        fake.state.borrow_mut().maps.insert(2, foreign.clone());
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        assert!(lifecycle.apply(&plan()).is_err());
        assert!(fake.state.borrow().maps[&1].is_empty());
        assert_eq!(fake.state.borrow().maps[&2], foreign);
        assert!(!fake.journal.exists());
    }

    #[test]
    fn local_hidutil_foreign_change_between_read_and_write_is_not_overwritten() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let foreign = compile(&[rule("a", "b")]).unwrap();
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        fake.state.borrow_mut().concurrent = Some(foreign.clone());
        assert!(lifecycle.apply(&plan()).is_err());
        assert_eq!(fake.state.borrow().maps[&1], foreign);
        assert_eq!(fake.state.borrow().writes, 0);
        assert!(!fake.journal.exists());
    }

    #[test]
    fn local_hidutil_failed_restore_retains_the_recovery_record_until_it_succeeds() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        lifecycle.apply(&plan()).unwrap();
        fake.state.borrow_mut().refuse = true;
        assert!(lifecycle.restore().is_err());
        assert!(fake.journal.exists());
        assert!(Lifecycle::recover(fake.clone(), &folder.0).is_err());
        fake.state.borrow_mut().refuse = false;
        Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        assert!(fake.state.borrow().maps.values().all(Vec::is_empty));
    }

    #[test]
    fn local_hidutil_device_scope_only_changes_matching_keyboards_and_disconnect_recovery_is_safe() {
        let folder = Folder::new();
        let fake = FakeHidutil::new(&folder);
        let mut lifecycle = Lifecycle::recover(fake.clone(), &folder.0).unwrap();
        lifecycle.apply(&Plan { devices: vec![(keyboard(2), plan().base)], ..Default::default() }).unwrap();
        assert!(fake.state.borrow().maps[&1].is_empty());
        assert_eq!(fake.state.borrow().maps[&2], plan().base);
        fake.state.borrow_mut().maps.remove(&2);
        lifecycle.restore().unwrap();
        assert!(!fake.journal.exists());
    }

    #[test]
    fn local_hidutil_partition_removes_system_sources_from_the_tap_and_keeps_chords() {
        let caps = rule("capslock", "esc");
        let chord = rule("a", "ctrl+c");
        let (system, tap) = partition(&[caps.clone(), chord.clone()]);
        assert_eq!(system, compile(&[caps]).unwrap());
        assert_eq!(tap.len(), 1);
        assert_eq!(tap[0].from, chord.from);
        let (system, tap) = partition(&[rule("capslock", "esc"), rule("esc", "ctrl+c")]);
        assert!(system.is_empty());
        assert_eq!(tap.len(), 2);
    }

    fn rule(from: &str, to: &str) -> RuleSpec {
        RuleSpec { from: from.parse().unwrap(), layer: None, what: What::To(to.parse().unwrap()) }
    }

    #[test]
    fn local_hidutil_caps_to_escape_uses_documented_keyboard_usages() {
        let mappings = compile(&[rule("capslock", "esc")]).unwrap();
        assert_eq!(mappings, [Mapping { source: 0x700000039, destination: 0x700000029 }]);
        assert_eq!(property(&mappings), "{\"UserKeyMapping\":[{\"HIDKeyboardModifierMappingSrc\":30064771129,\"HIDKeyboardModifierMappingDst\":30064771113}]}");
    }

    #[test]
    fn local_hidutil_caps_to_control_and_swaps_do_not_use_a_function_key_alias() {
        assert_eq!(compile(&[rule("capslock", "ctrl")]).unwrap(),
            [Mapping { source: 0x700000039, destination: 0x7000000e0 }]);
        let mappings = compile(&[rule("a", "b"), rule("b", "a")]).unwrap();
        assert_eq!(mappings, [Mapping { source: 0x700000004, destination: 0x700000005 },
            Mapping { source: 0x700000005, destination: 0x700000004 }]);
    }

    #[test]
    fn local_hidutil_generator_preserves_first_rule_priority_and_empty_properties() {
        assert_eq!(compile(&[rule("a", "b"), rule("a", "c")]).unwrap(), compile(&[rule("a", "b")]).unwrap());
        assert_eq!(property(&compile(&[]).unwrap()), "{\"UserKeyMapping\":[]}");
    }

    #[test]
    fn local_hidutil_generator_refuses_complex_or_ambiguous_rules() {
        assert_eq!(compile(&[rule("ctrl+a", "b")]), Err(Unsupported::ConditionalRule));
        assert_eq!(compile(&[rule("a", "ctrl+b")]), Err(Unsupported::ComplexOutput));
        assert_eq!(compile(&[rule("button4", "esc")]), Err(Unsupported::UnknownKey(0x113)));
        assert_eq!(compile(&[rule("backslash", "a")]), Err(Unsupported::AmbiguousKey(43)));
        let mut layered = rule("a", "b");
        layered.layer = Some("nav".into());
        assert_eq!(compile(&[layered]), Err(Unsupported::ConditionalRule));
        let mut dual = rule("capslock", "esc");
        dual.what = What::TapHold { tap: "esc".parse().unwrap(), hold: "ctrl".parse().unwrap(), timeout: None };
        assert_eq!(compile(&[dual]), Err(Unsupported::ComplexOutput));
    }
}
