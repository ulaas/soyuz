//! A confirmation before a command (`FrontendCmd::Confirm`): the command runs once, when
//! the user confirms; cancelling drops it.

use crate::cmd::Cmd;

/// What the confirmation shows and the command it guards.
#[derive(Debug)]
pub struct ConfirmModel {
    pub title: String,
    pub message: String,
    /// The confirm button's label.
    pub confirm: String,
    on_confirm: Option<Cmd>,
}

impl ConfirmModel {
    pub fn new(
        title: impl Into<String>,
        message: impl Into<String>,
        confirm: impl Into<String>,
        on_confirm: Cmd,
    ) -> Self {
        ConfirmModel {
            title: title.into(),
            message: message.into(),
            confirm: confirm.into(),
            on_confirm: Some(on_confirm),
        }
    }

    /// Confirmed: the command to queue (only the first time).
    pub fn confirm(&mut self) -> Option<Cmd> {
        self.on_confirm.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_runs_once() {
        let mut m = ConfirmModel::new("T", "M", "Do it", Cmd::CopyText("x".into()));
        assert!(matches!(m.confirm(), Some(Cmd::CopyText(t)) if t == "x"));
        assert!(m.confirm().is_none());
    }
}
