use super::*;
use super::pages::card;

impl App {
    pub(super) fn typing_card(&mut self, ui: &mut Ui, area: Rect) {
        let content = card(ui, area, "Typing on the other computer");
        let choices = [("physical", "Keys"), ("characters", "Letters"), ("auto", "Auto")];
        let track = Rect::from_min_size(pos2(content.left(), content.top()), vec2(content.width(), 28.0));
        let labels: Vec<_> = choices.iter().map(|(_, label)| *label).collect();
        let pills = Pills {
            id: Id::unique("typing-mode"),
            track,
            cells: Pills::even(track, 3, 2.0),
            labels: &labels,
            names: choices.iter().map(|(_, label)| format!("Typing preference: {label}")).collect(),
            chosen: choices.iter().position(|(value, _)| *value == self.prefs.typing),
            enabled: vec![true; 3],
            role: Role::RadioButton,
            font: T_META,
        };
        if let Some(index) = pills.show(ui, self.arrowed) {
            let mode = choices[index].0;
            self.set_pref("typing preference", &["typing", "--keys", mode], |prefs| prefs.typing = mode);
        }
        let what = match self.prefs.typing {
            "characters" => "The other computer types the letters printed on your keys, even with a different layout. Shortcuts still use the keys.",
            "auto" => "Letters when the two keyboard layouts differ, the same keys when they match.",
            _ => "Each key you press is the same key on the other computer, whatever its layout.",
        };
        let note = if cfg!(target_os = "linux") && self.prefs.typing != "physical" { " Letters work when you control a Mac or Windows computer." } else { "" };
        para(ui, pos2(content.left(), content.top() + 40.0), content.width(), &format!("{what}{note} It changes the next time the pointer crosses."), regular(T_META), INK2);
    }
}
