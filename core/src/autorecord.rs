//! Решения монитора звонков без доступа к аудио или GUI.

#[derive(Debug, PartialEq)]
pub enum AutoRecordAction {
    None,
    Start,
    Stop,
}

#[derive(Default)]
pub struct AutoRecordState {
    auto_active: bool,
    active_streak: u32,
    suppressed: bool,
}

impl AutoRecordState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn started(&mut self) {
        self.auto_active = true;
    }

    pub fn tick(
        &mut self,
        call: bool,
        recording: bool,
        manual_stop: bool,
        required: u32,
        auto_stop: bool,
    ) -> AutoRecordAction {
        if !recording {
            self.auto_active = false;
        }
        if manual_stop {
            self.suppressed = true;
            self.auto_active = false;
        }
        if !call {
            self.active_streak = 0;
            self.suppressed = false;
            if recording && self.auto_active && auto_stop {
                self.auto_active = false;
                return AutoRecordAction::Stop;
            }
        } else {
            self.active_streak = self.active_streak.saturating_add(1);
            if !recording && !self.suppressed && self.active_streak >= required.max(1) {
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
            state.tick(true, false, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, false, false, 2, true),
            AutoRecordAction::Start
        );
        state.started();
        assert_eq!(
            state.tick(true, false, true, 2, true),
            AutoRecordAction::None
        );
        for _ in 0..5 {
            assert_eq!(
                state.tick(true, false, false, 2, true),
                AutoRecordAction::None
            );
        }
        assert_eq!(
            state.tick(false, false, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, false, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, false, false, 2, true),
            AutoRecordAction::Start
        );
    }

    #[test]
    fn only_a_successfully_automatic_recording_can_be_stopped_automatically() {
        let mut state = AutoRecordState::default();
        assert_eq!(
            state.tick(true, false, false, 1, true),
            AutoRecordAction::Start
        );
        // Another control started a manual recording before the monitor could.
        assert_eq!(
            state.tick(false, true, false, 1, true),
            AutoRecordAction::None
        );
        state.started();
        assert_eq!(
            state.tick(false, true, false, 1, false),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(false, true, false, 1, true),
            AutoRecordAction::Stop
        );
    }

    #[test]
    fn disabling_monitor_resets_suppression_and_streak() {
        let mut state = AutoRecordState::default();
        state.tick(true, false, true, 1, true);
        state.reset();
        assert_eq!(
            state.tick(true, false, false, 2, true),
            AutoRecordAction::None
        );
        assert_eq!(
            state.tick(true, false, false, 2, true),
            AutoRecordAction::Start
        );
    }

    #[test]
    fn manual_stop_clears_ownership_even_if_recording_snapshot_is_stale() {
        let mut state = AutoRecordState::default();
        state.started();
        assert_eq!(
            state.tick(false, true, true, 1, true),
            AutoRecordAction::None
        );
    }
}
