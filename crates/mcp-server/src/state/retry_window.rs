use std::time::{Duration, Instant};

pub(crate) const DEFAULT_RETRY_BUDGET: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryOwner {
    Startup,
    ChangeHub,
    Drift,
    OverlayEmbedding,
    Graph,
}

impl RetryOwner {
    fn may_rearm_on_fresh_work(self) -> bool {
        !matches!(self, Self::Startup)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryStop {
    Exhausted,
    OperationError,
    Terminal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    RetryAfter(Duration),
    Stop(RetryStop),
}

#[cfg_attr(test, derive(Clone))]
pub(crate) struct RetryWindow {
    owner: RetryOwner,
    budget: Duration,
    deadline: Option<Instant>,
    streak: u32,
    stopped: Option<RetryStop>,
}

impl RetryWindow {
    pub(crate) fn new(owner: RetryOwner) -> Self {
        Self::with_budget(owner, DEFAULT_RETRY_BUDGET)
    }

    pub(crate) fn with_budget(owner: RetryOwner, budget: Duration) -> Self {
        Self { owner, budget, deadline: None, streak: 0, stopped: None }
    }

    pub(crate) fn streak(&self) -> u32 {
        self.streak
    }

    pub(crate) fn refused(&mut self, now: Instant, delay: Duration) -> RetryDecision {
        if let Some(reason) = self.stopped {
            return RetryDecision::Stop(reason);
        }

        let deadline = match self.deadline {
            Some(deadline) => deadline,
            None => match now.checked_add(self.budget) {
                Some(deadline) => {
                    self.deadline = Some(deadline);
                    deadline
                }
                None => return self.stop(RetryStop::Exhausted),
            },
        };
        let Some(remaining) = deadline.checked_duration_since(now) else {
            return self.stop(RetryStop::Exhausted);
        };
        if remaining.is_zero() {
            return self.stop(RetryStop::Exhausted);
        }

        self.streak = self.streak.saturating_add(1);
        RetryDecision::RetryAfter(delay.min(remaining))
    }

    /// The delay for a refusal ALREADY counted: the budget still bounds it, but the streak
    /// does not move twice for one refusal. A caller that counts a refusal where it happens and
    /// then asks here for the pause is the ordinary shape, and counting both doubles the
    /// backoff against the schedule every other owner follows.
    pub(crate) fn refused_again(&mut self, now: Instant, delay: Duration) -> RetryDecision {
        let streak = self.streak;
        let decision = self.refused(now, delay);
        self.streak = streak;
        decision
    }

    /// Whether another attempt may still be made at `now`, asked without spending anything:
    /// [`Self::refused`] answers the same question but counts itself as a refusal.
    pub(crate) fn is_open(&self, now: Instant) -> bool {
        self.stopped.is_none() && self.deadline.is_none_or(|deadline| now < deadline)
    }

    pub(crate) fn expired(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.stop(RetryStop::Exhausted);
            true
        } else {
            false
        }
    }

    pub(crate) fn complete(&mut self) {
        self.deadline = None;
        self.streak = 0;
        self.stopped = None;
    }

    /// Returns true only when fresh external work starts a new obligation.
    ///
    /// `now` matters: a deadline that has passed is exhausted whether or not anything has
    /// asked since. Reading only the latched `stopped` would drop the FIRST fresh signal after
    /// expiry — the one that arrives before any caller has had reason to call `refused`.
    pub(crate) fn observe_external_work(&mut self, now: Instant, genuinely_fresh: bool) -> bool {
        // Terminal is terminal: an expired deadline says the budget ran out, never that a
        // permanent stop may be reconsidered.
        let expired = !matches!(self.stopped, Some(RetryStop::Terminal))
            && self.deadline.is_some_and(|deadline| now >= deadline);
        if genuinely_fresh
            && (expired
                || matches!(self.stopped, Some(RetryStop::Exhausted | RetryStop::OperationError)))
            && self.owner.may_rearm_on_fresh_work()
        {
            self.deadline = None;
            self.streak = 0;
            self.stopped = None;
            return true;
        }
        false
    }

    pub(crate) fn operation_error(&mut self) -> RetryDecision {
        self.stop(RetryStop::OperationError)
    }

    pub(crate) fn terminal(&mut self) -> RetryDecision {
        self.stop(RetryStop::Terminal)
    }

    fn stop(&mut self, reason: RetryStop) -> RetryDecision {
        self.stopped = Some(reason);
        RetryDecision::Stop(reason)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_OWNERS: [RetryOwner; 5] = [
        RetryOwner::Startup,
        RetryOwner::ChangeHub,
        RetryOwner::Drift,
        RetryOwner::OverlayEmbedding,
        RetryOwner::Graph,
    ];
    const BACKGROUND_OWNERS: [RetryOwner; 4] =
        [RetryOwner::ChangeHub, RetryOwner::Drift, RetryOwner::OverlayEmbedding, RetryOwner::Graph];

    /// The vector publication counts a refusal where it happens and asks for the pause it
    /// then takes afterwards, so the same refusal passes through the window twice. It may
    /// move the schedule only once: counted at both places, every step of the backoff comes
    /// out doubled against the schedule every other retry owner follows.
    #[test]
    fn one_refusal_moves_the_schedule_one_step() {
        use crate::state::overlay_retry::retry_delay;

        let start = Instant::now();
        let mut window =
            RetryWindow::with_budget(RetryOwner::OverlayEmbedding, Duration::from_secs(3600));
        let mut pauses = Vec::new();
        for _ in 0..4 {
            // Where the refusal happened.
            let RetryDecision::RetryAfter(_) = window.refused(start, retry_delay(window.streak()))
            else {
                panic!("the budget ran out; this test is about the schedule, not the budget")
            };
            let counted = window.streak();
            // And the pause the same refusal then takes.
            let RetryDecision::RetryAfter(pause) =
                window.refused_again(start, retry_delay(window.streak()))
            else {
                panic!("the budget ran out between the two halves of one refusal")
            };
            assert_eq!(window.streak(), counted, "the second half counted the refusal again");
            pauses.push(pause);
        }

        assert_eq!(window.streak(), 4, "four refusals, four steps");
        assert_eq!(
            pauses,
            vec![retry_delay(1), retry_delay(2), retry_delay(3), retry_delay(4)],
            "the pause after the n-th refusal is the n-th step of the schedule"
        );
    }

    #[test]
    fn all_retry_owners_preserve_deadline_and_streak_when_coalescing() {
        let start = Instant::now();
        for owner in ALL_OWNERS {
            let mut window = RetryWindow::new(owner);
            assert_eq!(
                window.refused(start, Duration::from_secs(2)),
                RetryDecision::RetryAfter(Duration::from_secs(2))
            );
            let deadline = window.deadline;

            assert!(!window.observe_external_work(start, true), "{owner:?}");
            assert_eq!(window.deadline, deadline, "{owner:?}");
            assert_eq!(window.streak(), 1, "{owner:?}");
        }
    }

    #[test]
    fn eligible_background_owners_rearm_only_on_fresh_external_work() {
        let start = Instant::now();
        for owner in BACKGROUND_OWNERS {
            let mut window = RetryWindow::new(owner);
            assert!(matches!(window.refused(start, Duration::ZERO), RetryDecision::RetryAfter(_)));
            assert_eq!(
                window.refused(start + DEFAULT_RETRY_BUDGET, Duration::ZERO),
                RetryDecision::Stop(RetryStop::Exhausted)
            );

            assert!(!window.observe_external_work(start, false), "{owner:?}");
            assert!(window.observe_external_work(start, true), "{owner:?}");
            assert!(!window.observe_external_work(start, true), "{owner:?}");
            assert_eq!(window.streak(), 0, "{owner:?}");
            assert_eq!(window.deadline, None, "{owner:?}");
        }

        let mut startup = RetryWindow::new(RetryOwner::Startup);
        assert!(matches!(startup.refused(start, Duration::ZERO), RetryDecision::RetryAfter(_)));
        assert_eq!(
            startup.refused(start + DEFAULT_RETRY_BUDGET, Duration::ZERO),
            RetryDecision::Stop(RetryStop::Exhausted)
        );
        assert!(!startup.observe_external_work(start, true));
    }

    /// The first fresh signal after a deadline passes is the one most likely to be lost:
    /// nothing has called `refused` since, so the stop is not latched yet.
    #[test]
    fn first_fresh_work_after_expiry_rearms_the_owner() {
        let start = Instant::now();
        let mut window = RetryWindow::new(RetryOwner::Graph);
        assert!(matches!(window.refused(start, Duration::ZERO), RetryDecision::RetryAfter(_)));

        let late = start + DEFAULT_RETRY_BUDGET + Duration::from_secs(100);
        assert!(window.observe_external_work(late, true), "the first fresh signal was dropped");
        assert!(matches!(window.refused(late, Duration::ZERO), RetryDecision::RetryAfter(_)));
    }

    /// A terminal stop is not a budget that ran out; an expired deadline must not reopen it.
    #[test]
    fn a_terminal_stop_is_not_reopened_by_an_expired_deadline() {
        let start = Instant::now();
        let mut window = RetryWindow::new(RetryOwner::Graph);
        assert!(matches!(window.refused(start, Duration::ZERO), RetryDecision::RetryAfter(_)));
        window.terminal();

        let late = start + DEFAULT_RETRY_BUDGET + Duration::from_secs(100);
        assert!(!window.observe_external_work(late, true), "a terminal window was reopened");
    }

    #[test]
    fn all_retry_owners_stop_on_operation_error() {
        let now = Instant::now();
        for owner in ALL_OWNERS {
            let mut window = RetryWindow::new(owner);
            assert_eq!(
                window.operation_error(),
                RetryDecision::Stop(RetryStop::OperationError),
                "{owner:?}"
            );
            assert_eq!(
                window.refused(now, Duration::ZERO),
                RetryDecision::Stop(RetryStop::OperationError),
                "{owner:?}"
            );
            assert_eq!(window.observe_external_work(now, true), owner.may_rearm_on_fresh_work());

            let mut terminal = RetryWindow::new(owner);
            assert_eq!(terminal.terminal(), RetryDecision::Stop(RetryStop::Terminal));
            assert_eq!(
                terminal.refused(now, Duration::ZERO),
                RetryDecision::Stop(RetryStop::Terminal),
                "{owner:?}"
            );
        }
    }
}
