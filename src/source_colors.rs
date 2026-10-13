//! Session-stable container colors shared by resource markers and log labels.
use std::collections::HashMap;

const COLORS: [u32; 12] = [
    0x7eafe8, 0xe5a365, 0xbe9ae8, 0x74d7b1, 0xe87979, 0x73c9d0, 0xe5c07b, 0xe89bc5, 0xa8c975,
    0xa1c8ff, 0xd8b6ff, 0x9ce9ee,
];

#[derive(Default)]
pub(crate) struct SourceColors {
    names: HashMap<String, usize>,
}

impl SourceColors {
    pub(crate) fn register(&mut self, name: &str) {
        if !self.names.contains_key(name) {
            self.names.insert(name.to_owned(), self.names.len());
        }
    }

    pub(crate) fn color(&self, name: &str, background: u32) -> u32 {
        let index = self.names.get(name).copied().unwrap_or_default();
        crate::ansi::display_color(COLORS[index % COLORS.len()], background)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_distinct_and_survive_recreation_and_new_sources() {
        let mut colors = SourceColors::default();
        for name in ["project-web-1", "project-web-2", "project-worker-1"] {
            colors.register(name);
        }
        let original = colors.color("project-web-1", 0x101819);
        assert_ne!(original, colors.color("project-web-2", 0x101819));
        assert_ne!(original, colors.color("project-worker-1", 0x101819));
        colors.register("new-container");
        colors.register("project-web-1");
        assert_eq!(original, colors.color("project-web-1", 0x101819));
        assert_ne!(original, colors.color("project-web-1", 0xffffff));
    }
}
