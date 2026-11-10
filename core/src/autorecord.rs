//! Решения монитора звонков без доступа к аудио или GUI.

#[derive(Debug, PartialEq)]
pub enum AutoRecordAction {
    None,
    Start,
    Stop(String),
}

#[derive(Default)]
pub struct AutoRecordState {
    owned_id: Option<String>,
    active_streak: u32,
    suppressed: bool,
}

impl AutoRecordState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn started(&mut self, id: String) {
        self.owned_id = Some(id);
    }

    pub fn tick(
        &mut self,
        call: bool,
        recording_id: Option<&str>,
        manual_stop: bool,
        required: u32,
        auto_stop: bool,
    ) -> AutoRecordAction {
        if self.owned_id.as_deref() != recording_id {
            self.owned_id = None;
        }
        if manual_stop {
            self.suppressed = true;
            self.owned_id = None;
        }
        if !call {
            self.active_streak = 0;
            self.suppressed = false;
            if auto_stop {
                if let Some(id) = self.owned_id.take() {
                    return AutoRecordAction::Stop(id);
                }
            }
        } else {
            self.active_streak = self.active_streak.saturating_add(1);
            if recording_id.is_none() && !self.suppressed && self.active_streak >= required.max(1) {
                return AutoRecordAction::Start;
            }
        }
        AutoRecordAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_stop_blocks_the_current_call_but_allows_the_next_one_after_delay() {
        let mut state = AutoRecordState::default();
        assert_eq!(
            state.tick(true, None, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, None, false, 2, true),
            AutoRecordAction::Start
        );
        state.started("auto".into());
        assert_eq!(
            state.tick(true, None, true, 2, true),
            AutoRecordAction::None
        );
        for _ in 0..5 {
            assert_eq!(
                state.tick(true, None, false, 2, true),
                AutoRecordAction::None
            );
        }
        assert_eq!(
            state.tick(false, None, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, None, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, None, false, 2, true),
            AutoRecordAction::Start
        );
    }

    #[test]
    fn only_a_successfully_automatic_recording_can_be_stopped_automatically() {
        let mut state = AutoRecordState::default();
        assert_eq!(
            state.tick(true, None, false, 1, true),
            AutoRecordAction::Start
        );
        // Another control started a manual recording before the monitor could.
        assert_eq!(
            state.tick(false, Some("auto"), false, 1, true),
            AutoRecordAction::None
        );
        state.started("auto".into());
        assert_eq!(
            state.tick(false, Some("auto"), false, 1, false),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(false, Some("auto"), false, 1, true),
            AutoRecordAction::Stop("auto".into())
        );
    }

    #[test]
    fn disabling_monitor_resets_suppression_and_streak() {
        let mut state = AutoRecordState::default();
        state.tick(true, None, true, 1, true);
        state.reset();
        assert_eq!(
            state.tick(true, None, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, None, false, 2, true),
            AutoRecordAction::Start
        );
    }

    #[test]
    fn manual_stop_clears_ownership_even_if_recording_snapshot_is_stale() {
        let mut state = AutoRecordState::default();
        state.started("auto".into());
        assert_eq!(
            state.tick(false, Some("auto"), true, 1, true),
            AutoRecordAction::None
        );
    }

    #[test]
    fn stop_actions_carry_the_owned_id_and_never_target_a_replacement_recording() {
        let mut state = AutoRecordState::default();
        state.started("old-auto".into());
        assert_eq!(
            state.tick(false, Some("new-manual"), false, 1, true),
            AutoRecordAction::None
        );
        state.started("old-auto".into());
        assert_eq!(
            state.tick(false, Some("old-auto"), false, 1, true),
            AutoRecordAction::Stop("old-auto".into())
        );
    }
}
