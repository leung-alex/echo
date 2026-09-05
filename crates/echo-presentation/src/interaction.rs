//! Keyboard policy. Native input-method composition always wins over shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Search,
    Row,
    Control,
    Surface,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    None,
    Escape,
    FocusSearch(bool),
    SwitchPanel,
    Move(i32),
    Select(usize),
    ToggleBatch,
    SelectAllBatch,
    Primary,
    Copy,
    PreventDefault,
}
#[derive(Debug, Clone, Copy)]
pub struct Key<'a> {
    pub text: &'a str,
    pub ctrl: bool,
    pub shift: bool,
    pub composing: bool,
    pub target: Target,
    pub text_edit: bool,
    pub batch: bool,
}
pub fn interpret(key: Key<'_>) -> Intent {
    if key.composing {
        return Intent::None;
    }
    let k = key.text.to_lowercase();
    if key.ctrl && k == "f" {
        return Intent::FocusSearch(true);
    }
    if key.text == "Escape" {
        return Intent::Escape;
    }
    // Editor controls own text navigation, selection, space and Enter.
    if key.target == Target::Control {
        return Intent::None;
    }
    if key.batch {
        if key.ctrl && k == "a" {
            return Intent::SelectAllBatch;
        }
        if key.text == " " || key.text == "Enter" {
            return Intent::ToggleBatch;
        }
    }
    if key.text == "Tab" && key.target != Target::Row {
        return Intent::SwitchPanel;
    }
    if key.ctrl && !key.text_edit && (k == "h" || k == "l") {
        return Intent::SwitchPanel;
    }
    if key.ctrl && k == "j" {
        return Intent::Move(1);
    }
    if key.ctrl && k == "k" {
        return Intent::Move(-1);
    }
    if key.text == "ArrowUp" {
        return Intent::Move(-1);
    }
    if key.text == "ArrowDown" {
        return Intent::Move(1);
    }
    if key.target != Target::Search {
        if k == "k" && !key.ctrl {
            return Intent::Move(-1);
        }
        if (k == "j" || k == "i") && !key.ctrl {
            return Intent::Move(1);
        }
        if let Ok(n) = k.parse::<usize>() {
            if (1..=9).contains(&n) && !key.ctrl {
                return Intent::Select(n - 1);
            }
        }
        if k == "/" {
            return Intent::FocusSearch(false);
        }
        if key.ctrl && k == "c" {
            return Intent::Copy;
        }
    }
    if key.text == "Enter" {
        return Intent::Primary;
    }
    if key.target == Target::Search
        && !key.text_edit
        && (matches!(key.text, "ArrowLeft" | "ArrowRight" | "Home" | "End")
            || (key.ctrl && k == "a"))
    {
        return Intent::PreventDefault;
    }
    Intent::None
}
#[cfg(test)]
mod tests {
    use super::*;
    fn k(text: &str) -> Key<'_> {
        Key {
            text,
            ctrl: false,
            shift: false,
            composing: false,
            target: Target::Search,
            text_edit: false,
            batch: false,
        }
    }
    #[test]
    fn ime_never_triggers_an_action() {
        for text in ["Enter", "Escape", "Tab", "j", "ArrowDown"] {
            let mut x = k(text);
            x.composing = true;
            assert_eq!(interpret(x), Intent::None);
        }
    }
    #[test]
    fn normal_letters_are_typed_in_search() {
        for text in ["j", "k", "i", "1", "/"] {
            assert_eq!(interpret(k(text)), Intent::None);
        }
    }
    #[test]
    fn arrows_and_enter_navigate() {
        assert_eq!(interpret(k("ArrowUp")), Intent::Move(-1));
        assert_eq!(interpret(k("ArrowDown")), Intent::Move(1));
        assert_eq!(interpret(k("Enter")), Intent::Primary);
    }
    #[test]
    fn control_f_selects_search() {
        let mut x = k("f");
        x.ctrl = true;
        assert_eq!(interpret(x), Intent::FocusSearch(true));
    }
    #[test]
    fn text_editor_retains_its_keys() {
        for text in ["Enter", "ArrowDown", " ", "a", "j"] {
            let mut x = k(text);
            x.target = Target::Control;
            x.ctrl = true;
            assert_eq!(interpret(x), Intent::None);
        }
    }
    #[test]
    fn search_editing_mode_keeps_caret_keys() {
        for text in ["Home", "End", "ArrowLeft", "ArrowRight"] {
            let mut x = k(text);
            assert_eq!(interpret(x), Intent::PreventDefault);
            x.text_edit = true;
            assert_eq!(interpret(x), Intent::None);
        }
    }
    #[test]
    fn row_shortcuts_and_digits() {
        let mut x = k("9");
        x.target = Target::Row;
        assert_eq!(interpret(x), Intent::Select(8));
        x.text = "k";
        assert_eq!(interpret(x), Intent::Move(-1));
        x.text = "/";
        assert_eq!(interpret(x), Intent::FocusSearch(false));
    }
    #[test]
    fn batch_enter_does_not_paste() {
        let mut x = k("Enter");
        x.batch = true;
        assert_eq!(interpret(x), Intent::ToggleBatch);
        x.text = "a";
        x.ctrl = true;
        assert_eq!(interpret(x), Intent::SelectAllBatch);
    }
    #[test]
    fn tab_on_button_row_does_not_switch_panel() {
        let mut x = k("Tab");
        x.target = Target::Row;
        assert_eq!(interpret(x), Intent::None);
        x.target = Target::Control;
        assert_eq!(interpret(x), Intent::None);
    }
}
