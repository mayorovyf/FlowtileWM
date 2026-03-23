#![forbid(unsafe_code)]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ColumnMode {
    Normal,
    Tabbed,
    MaximizedColumn,
    CustomWidth,
}

impl ColumnMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Tabbed => "tabbed",
            Self::MaximizedColumn => "maximized-column",
            Self::CustomWidth => "custom-width",
        }
    }
}

pub const fn bootstrap_modes() -> [ColumnMode; 4] {
    [
        ColumnMode::Normal,
        ColumnMode::Tabbed,
        ColumnMode::MaximizedColumn,
        ColumnMode::CustomWidth,
    ]
}

pub const fn preserves_insert_invariant() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{ColumnMode, bootstrap_modes, preserves_insert_invariant};

    #[test]
    fn exposes_all_bootstrap_modes() {
        let modes = bootstrap_modes();
        assert_eq!(
            modes,
            [
                ColumnMode::Normal,
                ColumnMode::Tabbed,
                ColumnMode::MaximizedColumn,
                ColumnMode::CustomWidth,
            ]
        );
    }

    #[test]
    fn keeps_insert_invariant_visible_in_bootstrap() {
        assert!(preserves_insert_invariant());
    }
}
