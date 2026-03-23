use core::fmt;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}", self.0)
            }
        }
    };
}

define_id!(MonitorId);
define_id!(WorkspaceSetId);
define_id!(WorkspaceId);
define_id!(ColumnId);
define_id!(WindowId);
define_id!(CorrelationId);
