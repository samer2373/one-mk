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
            names: choices.iter().map(|(value, label)| {
                if *value == "physical" { format!("Typing preference: {label}") } else { format!("Typing preference: {label} (experimental)") }
            }).collect(),
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
            "characters" => "Experimental: the other computer types the letters printed on your keys, even with a different layout. Only Windows and Mac computers can, untested on real hardware; Linux uses the keys.",
            "auto" => "Experimental: letters when the two keyboard layouts differ, the same keys when they match. Only Windows and Mac computers can type letters; Linux uses the keys.",
            _ => "Each key you press is the same key on the other computer, whatever its layout. Letters and Auto are experimental.",
        };
        para(ui, pos2(content.left(), content.top() + 40.0), content.width(), &format!("{what} It changes the next time the pointer crosses."), regular(T_META), pal().ink2);
    }
}
