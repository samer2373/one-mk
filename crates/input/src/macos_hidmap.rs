use crate::devcorr::hid_key_to_evdev;
use one_mk_proto::remap::{Output, RuleSpec, What};
use std::collections::BTreeMap;

const KEYBOARD_PAGE: u64 = 0x0007_0000_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub source: u64,
    pub destination: u64,
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
