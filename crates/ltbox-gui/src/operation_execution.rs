//! Ownership and progress of the foreground operation, independent of wizard UI state.
use crate::{OpStep, OperationPhaseKind, PhaseReporter, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OperationId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OperationKind {
    Unphased,
    Phased(OperationPhaseKind),
    Cleanup,
    SelfUpdate,
}

#[derive(Debug)]
struct ActiveOperation {
    id: OperationId,
    view: Option<View>,
    kind: OperationKind,
    reporter: Option<PhaseReporter>,
    completion_bound: bool,
}

#[derive(Debug)]
pub(crate) struct OperationExecution {
    generation: u64,
    active: Option<ActiveOperation>,
    pub(crate) steps: Vec<OpStep>,
    completed_step: usize,
    writes_started: bool,
    started_at: Option<std::time::Instant>,
    elapsed: std::time::Duration,
    pub(crate) direct_update: crate::DirectUpdateState,
}

impl OperationExecution {
    pub(crate) fn start(
        &mut self,
        view: Option<View>,
        kind: OperationKind,
        reporter: Option<PhaseReporter>,
    ) {
        assert!(self.active.is_none(), "an operation already owns execution");
        self.generation = self
            .generation
            .checked_add(1)
            .expect("operation ID exhausted");
        self.steps = reporter
            .as_ref()
            .map(PhaseReporter::steps)
            .unwrap_or_default();
        self.completed_step = 0;
        self.writes_started = false;
        self.started_at = Some(std::time::Instant::now());
        self.elapsed = std::time::Duration::ZERO;
        self.active = Some(ActiveOperation {
            id: OperationId(self.generation),
            view,
            kind,
            reporter,
            completion_bound: false,
        });
    }

    pub(crate) fn id(&self) -> Option<OperationId> {
        self.active.as_ref().map(|active| active.id)
    }
    /// Bind once at the innermost dispatcher that started this operation.
    /// Outer deferred-input batches also contain unrelated background tasks.
    pub(crate) fn bind_completion(&mut self) -> Option<OperationId> {
        let active = self.active.as_mut()?;
        if active.completion_bound {
            return None;
        }
        active.completion_bound = true;
        Some(active.id)
    }
    pub(crate) fn is_running(&self) -> bool {
        // Existing device-work UI treats direct updates separately. They still
        // own the same reservation and receive generation-checked completions.
        self.active
            .as_ref()
            .is_some_and(|active| active.kind != OperationKind::SelfUpdate)
    }
    pub(crate) fn view(&self) -> Option<View> {
        self.active.as_ref().and_then(|active| active.view)
    }
    pub(crate) fn phase_kind(&self) -> Option<OperationPhaseKind> {
        match self.active.as_ref()?.kind {
            OperationKind::Phased(kind) => Some(kind),
            _ => None,
        }
    }
    pub(crate) fn current_step(&self) -> usize {
        self.active
            .as_ref()
            .and_then(|active| active.reporter.as_ref())
            .map(PhaseReporter::current_step)
            .unwrap_or(self.completed_step)
    }
    pub(crate) fn writes_started(&self) -> bool {
        self.writes_started
            || self
                .active
                .as_ref()
                .and_then(|active| active.reporter.as_ref())
                .is_some_and(PhaseReporter::writes_started)
    }
    pub(crate) fn elapsed(&self) -> std::time::Duration {
        self.started_at
            .map_or(self.elapsed, |started_at| started_at.elapsed())
    }
    pub(crate) fn finish(&mut self, success: bool) {
        self.completed_step = if success && !self.steps.is_empty() {
            self.steps.len() - 1
        } else {
            self.current_step()
        };
        self.writes_started = self.writes_started();
        self.elapsed = self.elapsed();
        self.started_at = None;
        self.active = None;
    }
    pub(crate) fn set_completed_step(&mut self, step: usize) {
        debug_assert!(!self.is_running());
        self.completed_step = step.min(self.steps.len().saturating_sub(1));
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        busy: bool,
        view: Option<View>,
        steps: Vec<OpStep>,
        step: usize,
        kind: Option<OperationPhaseKind>,
    ) -> Self {
        let mut execution = Self::default();
        if busy {
            execution.start(
                view,
                kind.map(OperationKind::Phased)
                    .unwrap_or(OperationKind::Unphased),
                None,
            );
        }
        execution.steps = steps;
        execution.completed_step = step;
        execution
    }
}

impl Default for OperationExecution {
    fn default() -> Self {
        Self {
            generation: 0,
            active: None,
            steps: Vec::new(),
            completed_step: 0,
            writes_started: false,
            started_at: None,
            elapsed: std::time::Duration::ZERO,
            direct_update: crate::DirectUpdateState::Ready,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, Message, RebootMsg};

    #[test]
    fn deferred_duplicate_starts_and_late_picker_leave_owner_unchanged() {
        let mut app = App::default();
        app.queries.poll_deferred.extend([
            Message::Settings(crate::SettingsMsg::CleanupTempFiles),
            Message::Settings(crate::SettingsMsg::CleanupTempFiles),
            Message::FlashPhys(crate::FlashPhysMsg::FlashPhysExecStart),
            Message::DumpPhys(crate::DumpPhysMsg::DumpPhysFolderChosen(Some(
                "late-picker".into(),
            ))),
        ]);
        let _ = app.resume_after_device_poll();
        assert!(app.cleaning_temp);
        assert!(app.operation.is_running());
        assert_eq!(app.operation.view(), None);
        assert!(app.queries.poll_deferred.is_empty());
        // The inner cleanup dispatcher consumed the binding. An outer poll
        // batch cannot bind unrelated driver/connectivity replies to its ID.
        assert_eq!(app.operation.bind_completion(), None);
        let id = app.operation.id().unwrap();
        let _ = app.update(Message::OperationEvent(
            id,
            Box::new(Message::Settings(crate::SettingsMsg::CleanupDone)),
        ));
        assert!(!app.operation.is_running());
        let _ = app.update(Message::ConnectivityChecked(true));
        assert_eq!(app.online, Some(true));
    }

    #[test]
    fn late_completion_cannot_finish_a_new_run_of_the_same_operation() {
        let mut app = App::default();
        app.begin_op(View::Reboot);
        let old = app.operation.id().unwrap();
        app.end_op();
        app.begin_op(View::Reboot);
        let current = app.operation.id().unwrap();
        let completion = |id| {
            Message::OperationEvent(
                id,
                Box::new(Message::Reboot(RebootMsg::RebootDone(Vec::new()))),
            )
        };
        let _ = app.update(completion(old));
        assert_eq!(app.operation.id(), Some(current));
        let _ = app.update(completion(current));
        assert!(!app.operation.is_running());
        // A duplicated completion cannot release a subsequent reservation.
        app.begin_silent_op(View::Root);
        let next = app.operation.id();
        let _ = app.update(completion(current));
        assert_eq!(app.operation.id(), next);
    }

    #[test]
    fn failure_retains_phase_and_write_attempt_but_next_run_is_isolated() {
        let mut app = App::default();
        let old = app.begin_phased_op(View::Root, OperationPhaseKind::Root);
        let _ = old.marker(6);
        assert!(!app.operation.writes_started());
        old.mark_writes_started();
        app.fail_op();
        assert_eq!(app.operation.current_step(), 5);
        assert!(app.operation.writes_started());
        let current = app.begin_phased_op(View::Root, OperationPhaseKind::Root);
        old.mark_writes_started();
        let _ = old.marker(8);
        assert_eq!(app.operation.current_step(), 0);
        assert!(!app.operation.writes_started());
        current.mark_writes_started();
        app.end_op();
        assert_eq!(app.operation.current_step(), 7);
        assert!(app.operation.writes_started());
    }

    #[test]
    fn stale_updater_failure_cannot_release_a_retry() {
        let mut app = App::default();
        app.operation.start(None, OperationKind::SelfUpdate, None);
        app.operation.direct_update = crate::DirectUpdateState::Updating;
        let old = app.operation.id().unwrap();
        let failure = |id| {
            Message::OperationEvent(
                id,
                Box::new(Message::SelfUpdateFinished(Err(crate::SelfUpdateFailure {
                    kind: crate::SelfUpdateFailureKind::Download,
                    detail: "test failure".into(),
                }))),
            )
        };
        let _ = app.update(failure(old));
        assert_eq!(app.operation.id(), None);
        app.operation.start(None, OperationKind::SelfUpdate, None);
        app.operation.direct_update = crate::DirectUpdateState::Updating;
        let retry = app.operation.id();
        let _ = app.update(failure(old));
        assert_eq!(app.operation.id(), retry);
        assert_eq!(
            app.operation.direct_update,
            crate::DirectUpdateState::Updating
        );
    }
}
