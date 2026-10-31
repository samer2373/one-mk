use super::*;
use super::pages::card;

impl App {
    pub(super) fn typing_card(&mut self, ui: &mut Ui, area: Rect) {
        let content = card(ui, area, "Typing on the other computer");
        let choices = [("physical", "Same keys"), ("characters", "Letters (not yet)"), ("auto", "Auto (not yet)")];
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
            radius: 6,
        };
        if let Some(index) = pills.show(ui, self.arrowed) {
            let mode = choices[index].0;
            self.set_pref("typing preference", &["typing", "--keys", mode], |prefs| prefs.typing = mode);
        }
        let effective = self.running().map(|status| status.typing.as_str()).filter(|typing| !typing.is_empty()).unwrap_or("physical (daemon not running; native character injection unavailable)");
        para(ui, pos2(content.left(), content.top() + 40.0), content.width(), &format!("Effective typing: {effective}. Changes apply on the next visit. Native character injection is not available yet."), regular(T_META), INK2);
    }
}
