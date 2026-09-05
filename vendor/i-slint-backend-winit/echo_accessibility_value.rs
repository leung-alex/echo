// Echo's narrowly scoped Slint 1.17.1 compatibility patch. The UI role,
// not a user's current text, determines the accessibility value type.
fn set_accessible_value(node: &mut accesskit::Node, role: accesskit::Role, value: &str) {
    if matches!(
        role,
        accesskit::Role::Slider | accesskit::Role::SpinButton | accesskit::Role::ProgressIndicator
    ) {
        if let Ok(number) = value.parse::<f64>() {
            node.set_numeric_value(number);
            return;
        }
    }
    node.set_value(value);
}
#[cfg(test)]
mod echo_accessibility_tests {
    use super::set_accessible_value;
    use accesskit::{Node, Role};
    #[test]
    fn numeric_search_and_text_keep_string_values_and_leading_zeroes() {
        for role in [
            Role::TextInput,
            Role::SearchInput,
            Role::NumberInput,
            Role::MultilineTextInput,
        ] {
            for text in ["0013", "0", "5000", "1e6", "", "123 ??"] {
                let mut node = Node::new(role);
                set_accessible_value(&mut node, role, text);
                assert_eq!(node.value(), Some(text));
                assert_eq!(node.numeric_value(), None);
            }
        }
    }
    #[test]
    fn actual_range_widgets_keep_numeric_values() {
        let mut node = Node::new(Role::Slider);
        set_accessible_value(&mut node, Role::Slider, "42");
        assert_eq!(node.numeric_value(), Some(42.0));
    }
}
