#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualState {
    Transit,
    Settled,
}

impl VisualState {
    pub fn shows_storeys(self) -> bool {
        self == Self::Settled
    }

    pub fn shows_file_menu(self) -> bool {
        self == Self::Settled
    }

    pub fn flight_scale(self) -> f32 {
        match self {
            Self::Transit => 1.0,
            Self::Settled => 0.55,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_view_is_static_and_close() {
        assert!(VisualState::Transit.flight_scale() > VisualState::Settled.flight_scale());
    }
}
