#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    ShowHide,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ToggleWindow,
    Shutdown,
}

pub struct TrayController<F>
where
    F: FnMut(TrayCommand),
{
    dispatch: F,
}

impl<F> TrayController<F>
where
    F: FnMut(TrayCommand),
{
    pub fn new(dispatch: F) -> Self {
        Self { dispatch }
    }

    pub fn dispatch(&mut self, action: TrayAction) {
        let command = match action {
            TrayAction::ShowHide => TrayCommand::ToggleWindow,
            TrayAction::Quit => TrayCommand::Shutdown,
        };
        (self.dispatch)(command);
    }
}
