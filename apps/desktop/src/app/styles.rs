use super::*;

impl App {
    pub(super) fn highlight_color(&self) -> String {
        if self.environment.high_contrast {
            String::new()
        } else {
            self.styles.match_color(self.window.get_dark())
        }
    }
    pub(super) fn apply_styles(&mut self) {
        let dark = self.window.get_dark();
        let theme = (dark, self.environment.high_contrast);
        if self.style_theme == Some(theme) {
            return;
        }
        self.style_theme = Some(theme);
        crate::style::apply_style!(
            self.window.global::<crate::DesignTokens>(),
            &self.styles,
            dark
        );
        // Restyle retained rows in place: no query/reload, identity, selection or scroll reset.
        let color = self.highlight_color();
        let query = self
            .surface
            .presented_query
            .as_deref()
            .unwrap_or(&self.surface.query);
        let recolor = |model: &dyn slint::Model<Data = crate::EntryRow>, query: &str| {
            (0..model.row_count())
                .filter_map(|index| {
                    let (row, bytes) = echo_windows::allocation::measure_owned(|| {
                        model.row_data(index).map(|mut row| {
                            // Re-own strings so allocation charges remain valid after the old
                            // row is replaced. Images remain owned by the thumbnail cache.
                            for text in [
                                &mut row.key,
                                &mut row.title,
                                &mut row.body,
                                &mut row.kind,
                                &mut row.time_label,
                                &mut row.section_label,
                                &mut row.icon_key,
                                &mut row.tags,
                            ] {
                                *text = text.as_str().to_owned().into();
                            }
                            let mut matcher = FuzzyMatcher::new(query);
                            crate::match_highlight::apply(&mut row, &mut matcher, &color);
                            row
                        })
                    });
                    row.map(|row| crate::native_model::OwnedRow { row, bytes })
                })
                .collect::<Vec<_>>()
        };
        self.model.reconcile(recolor(self.model.as_ref(), query));
        self.model_bytes = self.model.held_bytes();
        if self.window.get_outgoing_present() {
            let outgoing = self.window.get_outgoing();
            if let Some(model) = outgoing
                .rows
                .as_any()
                .downcast_ref::<crate::native_model::EntryModel>()
            {
                let before = model.held_bytes();
                model.reconcile(recolor(model, outgoing.query.as_str()));
                self.software.outgoing_bytes =
                    self.software.outgoing_bytes.saturating_sub(before) + model.held_bytes();
            }
        }
    }
}
