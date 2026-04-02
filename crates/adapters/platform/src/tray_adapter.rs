#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ShowHide,
    Quit,
}

pub struct TrayController<F>
where
    F: FnMut(&'static str),
{
    dispatch: F,
}

impl<F> TrayController<F>
where
    F: FnMut(&'static str),
{
    pub fn new(dispatch: F) -> Self {
        Self { dispatch }
    }

    pub fn dispatch(&mut self, action: TrayAction) {
        let command = match action {
            TrayAction::ShowHide => "toggle_window",
            TrayAction::Quit => "shutdown",
        };
        (self.dispatch)(command);
    }
}
