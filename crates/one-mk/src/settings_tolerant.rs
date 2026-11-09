//! Reading `settings.toml` around a mistake. One typo in a hand-edited file
//! (`natural = "maybe"`) must not switch off every other setting: the
//! section, or the single rule, that can't be read is left out, the rest is
//! kept, and each thing left out is reported with the line it is on.
//!
//! This is only for reading (the daemon, `status`, `doctor`, the listings).
//! Anything that *changes* the file still refuses a file it can't read in
//! full, so a hand edit is never overwritten by a copy with the broken part
//! missing.

use crate::settings::Settings;
use crate::tuning::{AutoscrollRule, ButtonRule, GestureRule, KeyRule, PointerRule, ScrollRule};
use serde::de::DeserializeOwned;

/// `text` without what can't be read, and what that was; `None` if the
/// file is not TOML at all (the plain error says where) or nothing needs
/// leaving out.
pub fn repair(text: &str) -> Option<(String, Vec<String>)> {
    let mut table: toml::Table = toml::from_str(text).ok()?;
    let mut problems = Vec::new();
    for key in table.keys().cloned().collect::<Vec<_>>() {
        if readable(&table, &key) {
            continue;
        }
        if key == "input" {
            repair_input(text, &mut table, &mut problems);
        } else {
            let why = why(&table, &key);
            table.remove(&key);
            problems.push(format!("{}[{key}] is ignored: {why}", at(section_line(text, &key))));
        }
    }
    if problems.is_empty() {
        return None;
    }
    Some((toml::to_string(&table).ok()?, problems))
}

/// Whether the file with only `table[key]` reads.
fn readable(table: &toml::Table, key: &str) -> bool {
    probe(table, key).try_into::<Settings>().is_ok()
}

fn why(table: &toml::Table, key: &str) -> String {
    match probe(table, key).try_into::<Settings>() {
        Err(error) => one_line(&error.to_string()),
        Ok(_) => String::new(),
    }
}

fn probe(table: &toml::Table, key: &str) -> toml::Value {
    let mut one = toml::Table::new();
    if let Some(value) = table.get(key) {
        one.insert(key.to_owned(), value.clone());
    }
    toml::Value::Table(one)
}

fn one_line(error: &str) -> String {
    error.lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("; ")
}

fn at(line: Option<usize>) -> String {
    line.map_or_else(|| "settings.toml: ".to_owned(), |line| format!("settings.toml line {line}: "))
}

fn compact(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The (1-based) line a `[key]` table or a `key =` value starts on.
fn section_line(text: &str, key: &str) -> Option<usize> {
    let heads = [format!("[{key}]"), format!("[{key}."), format!("[[{key}]]"), format!("[[{key}."), format!("{key}="), format!("{key}.")];
    text.lines().position(|line| {
        let line = compact(line);
        heads.iter().any(|head| line.starts_with(head.as_str()))
    }).map(|index| index + 1)
}

/// The line the `nth` (0-based) `[[path]]` entry starts on.
fn entry_line(text: &str, path: &str, nth: usize) -> Option<usize> {
    let head = format!("[[{path}]]");
    text.lines().enumerate().filter(|(_, line)| compact(line) == head).nth(nth).map(|(index, _)| index + 1)
}

/// Drops every entry of the array at `path` (`input.scroll`) that doesn't
/// read as a `T`.
fn prune<T: DeserializeOwned>(text: &str, array: Option<&mut toml::Value>, path: &str, problems: &mut Vec<String>) {
    let Some(entries) = array.and_then(toml::Value::as_array_mut) else { return };
    let mut index = 0;
    entries.retain(|entry| {
        let nth = index;
        index += 1;
        match entry.clone().try_into::<T>() {
            Ok(_) => true,
            Err(error) => {
                problems.push(format!("{}[[{path}]] entry {} is ignored: {}", at(entry_line(text, path, nth)), nth + 1, one_line(&error.to_string())));
                false
            }
        }
    });
}

fn repair_input(text: &str, table: &mut toml::Table, problems: &mut Vec<String>) {
    let Some(input) = table.get_mut("input").and_then(toml::Value::as_table_mut) else {
        table.remove("input");
        problems.push(format!("{}[input] is ignored: it must be a table", at(section_line(text, "input"))));
        return;
    };
    prune::<PointerRule>(text, input.get_mut("pointer"), "input.pointer", problems);
    prune::<ScrollRule>(text, input.get_mut("scroll"), "input.scroll", problems);
    prune::<ButtonRule>(text, input.get_mut("button"), "input.button", problems);
    prune::<GestureRule>(text, input.get_mut("gesture"), "input.gesture", problems);
    prune::<KeyRule>(text, input.get_mut("keymap").and_then(toml::Value::as_table_mut).and_then(|keymap| keymap.get_mut("rule")), "input.keymap.rule", problems);
    prune::<AutoscrollRule>(text, input.get_mut("autoscroll").and_then(toml::Value::as_table_mut).and_then(|auto| auto.get_mut("rule")), "input.autoscroll.rule", problems);
    if readable(table, "input") {
        return;
    }
    // What is left is wrong in a setting of its own (the base autoscroll
    // rule, `tap_ms`...): leave out those sub-sections, one by one.
    let keys: Vec<String> = table.get("input").and_then(toml::Value::as_table).map(|input| input.keys().cloned().collect()).unwrap_or_default();
    for key in keys {
        let mut one = toml::Table::new();
        let mut inner = toml::Table::new();
        if let Some(value) = table.get("input").and_then(toml::Value::as_table).and_then(|input| input.get(&key)) {
            inner.insert(key.clone(), value.clone());
        }
        one.insert("input".into(), toml::Value::Table(inner));
        if let Err(error) = toml::Value::Table(one).try_into::<Settings>() {
            if let Some(input) = table.get_mut("input").and_then(toml::Value::as_table_mut) {
                input.remove(&key);
            }
            problems.push(format!("{}[input.{key}] is ignored: {}", at(section_line(text, &format!("input.{key}")).or_else(|| section_line(text, "input"))), one_line(&error.to_string())));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_value_costs_its_rule_not_the_file() {
        let text = "notify_files = false\n\n[input.keymap]\ntap_ms = 180\n\n[[input.keymap.rule]]\nfrom = \"capslock\"\nto = \"esc\"\n\n[[input.scroll]]\nspeed = 2.0\n\n[[input.scroll]]\nnatural = \"maybe\"\n\n[[input.pointer]]\nspeed = 1.5\n";
        assert!(toml::from_str::<Settings>(text).is_err());
        let (fixed, problems) = repair(text).unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("settings.toml line 13: [[input.scroll]] entry 2 is ignored"), "{problems:?}");
        assert!(problems[0].contains("maybe"), "{problems:?}");
        let settings: Settings = toml::from_str(&fixed).unwrap();
        assert!(!settings.notify_files);
        assert_eq!(settings.input.keymap.rule.len(), 1);
        assert_eq!(settings.input.scroll.len(), 1);
        assert_eq!(settings.input.pointer.len(), 1);
        assert_eq!(settings.input.keymap.tap_ms, Some(180));
    }

    #[test]
    fn a_bad_section_is_left_out_with_its_line() {
        let text = "[typing]\nkeys = \"german\"\n\n[clipboard]\ntext = false\n";
        let (fixed, problems) = repair(text).unwrap();
        assert!(problems.iter().any(|problem| problem.starts_with("settings.toml line 1: [typing] is ignored")), "{problems:?}");
        let settings: Settings = toml::from_str(&fixed).unwrap();
        assert!(!settings.clipboard.text);
    }

    #[test]
    fn a_readable_file_or_a_syntax_error_is_left_to_the_plain_reader() {
        assert!(repair("notify_files = false\n").is_none());
        assert!(repair("this is = = not toml").is_none());
    }
}
