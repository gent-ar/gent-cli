#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectionPicker {
    Provider,
    Model,
    Effort,
    Mode,
    Permission,
}
