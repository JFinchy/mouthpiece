//! Single owner of "what is the microphone being used for right now".
//!
//! Every way of starting or stopping a recording — the hotkey, double-tap
//! Fn, the tray/widget button, deep links, voice enrollment, and meeting
//! recording — goes
//! through `CaptureController::request`. That replaces a bare
//! `is_recording` flag that each entry point flipped (and checked) on its
//! own, which let enrollment and dictation race each other and made
//! "start while already recording" behave differently per caller.

use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Hotkey,
    DoubleTapFn,
    /// The widget's toggle button (the `toggle_recording_command` IPC).
    Button,
    /// The tray menu's Start/Stop item.
    Tray,
    DeepLink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureState {
    Idle,
    Dictating {
        trigger: Trigger,
        session: u64,
    },
    Enrolling {
        session: u64,
    },
    /// Recording a meeting (see `meeting.rs`); dictation waits until it ends.
    Meeting {
        session: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    StartDictation(Trigger),
    StopDictation(Trigger),
    ToggleDictation(Trigger),
    /// Stop recording and throw it away (Esc, the widget's ✕).
    DiscardDictation,
    StartEnrollment,
    StopEnrollment,
    StartMeeting,
    StopMeeting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    StartedDictation { session: u64 },
    StoppedDictation { session: u64 },
    DiscardedDictation { session: u64 },
    StartedEnrollment { session: u64 },
    StoppedEnrollment { session: u64 },
    StartedMeeting { session: u64 },
    StoppedMeeting { session: u64 },
    Rejected(&'static str),
}

/// Pure state transition — no I/O, so every rule is unit-testable.
/// `next_session` is the id a newly started session would get.
fn transition(
    state: CaptureState,
    request: Request,
    next_session: u64,
) -> (CaptureState, Transition) {
    use CaptureState::*;
    use Request::*;

    match (state, request) {
        (Idle, StartDictation(trigger) | ToggleDictation(trigger)) => (
            Dictating {
                trigger,
                session: next_session,
            },
            Transition::StartedDictation {
                session: next_session,
            },
        ),
        (Dictating { session, .. }, StopDictation(_) | ToggleDictation(_)) => {
            (Idle, Transition::StoppedDictation { session })
        }
        (Dictating { session, .. }, DiscardDictation) => {
            (Idle, Transition::DiscardedDictation { session })
        }
        (Idle | Enrolling { .. }, DiscardDictation) => {
            (state, Transition::Rejected("not dictating"))
        }
        (Idle, StartEnrollment) => (
            Enrolling {
                session: next_session,
            },
            Transition::StartedEnrollment {
                session: next_session,
            },
        ),
        (Enrolling { session }, StopEnrollment) => {
            (Idle, Transition::StoppedEnrollment { session })
        }

        (Dictating { .. }, StartDictation(_)) => (state, Transition::Rejected("already dictating")),
        (Idle, StopDictation(_)) => (state, Transition::Rejected("not dictating")),
        (Idle, StopEnrollment) => (state, Transition::Rejected("not enrolling")),
        (Dictating { .. }, StartEnrollment | StopEnrollment) => (
            state,
            Transition::Rejected("microphone is in use for dictation"),
        ),
        (Enrolling { .. }, StartDictation(_) | StopDictation(_) | ToggleDictation(_)) => (
            state,
            Transition::Rejected("microphone is in use for voice enrollment"),
        ),
        (Enrolling { .. }, StartEnrollment) => (state, Transition::Rejected("already enrolling")),

        (Idle, StartMeeting) => (
            Meeting {
                session: next_session,
            },
            Transition::StartedMeeting {
                session: next_session,
            },
        ),
        (Meeting { session }, StopMeeting) => (Idle, Transition::StoppedMeeting { session }),
        (Meeting { .. }, StartMeeting) => {
            (state, Transition::Rejected("already recording a meeting"))
        }
        (Idle | Dictating { .. } | Enrolling { .. }, StopMeeting) => {
            (state, Transition::Rejected("not recording a meeting"))
        }
        (Dictating { .. }, StartMeeting) => (
            state,
            Transition::Rejected("microphone is in use for dictation"),
        ),
        (Enrolling { .. }, StartMeeting) => (
            state,
            Transition::Rejected("microphone is in use for voice enrollment"),
        ),
        (
            Meeting { .. },
            StartDictation(_) | StopDictation(_) | ToggleDictation(_) | DiscardDictation,
        ) => (state, Transition::Rejected("a meeting is being recorded")),
        (Meeting { .. }, StartEnrollment | StopEnrollment) => {
            (state, Transition::Rejected("a meeting is being recorded"))
        }
    }
}

pub struct CaptureController {
    inner: Mutex<Inner>,
}

struct Inner {
    state: CaptureState,
    next_session: u64,
}

impl CaptureController {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                state: CaptureState::Idle,
                next_session: 1,
            }),
        }
    }

    pub fn request(&self, request: Request) -> Transition {
        let mut inner = self.inner.lock().unwrap();
        let before = inner.state;
        let (after, result) = transition(before, request, inner.next_session);
        inner.state = after;
        if matches!(
            result,
            Transition::StartedDictation { .. }
                | Transition::StartedEnrollment { .. }
                | Transition::StartedMeeting { .. }
        ) {
            inner.next_session += 1;
        }
        drop(inner);

        match result {
            Transition::Rejected(reason) => {
                crate::applog!("capture: rejected {request:?} in {before:?}: {reason}")
            }
            _ => crate::applog!("capture: {before:?} -> {after:?} via {request:?}"),
        }
        result
    }

    pub fn state(&self) -> CaptureState {
        self.inner.lock().unwrap().state
    }

    /// True only while *this specific* session is still running. Per-session
    /// background loops (the level meter) poll this rather than a shared
    /// flag, so a quick stop-then-start can't leave the previous session's
    /// loop running alongside the new one.
    pub fn is_session_active(&self, session: u64) -> bool {
        match self.state() {
            CaptureState::Dictating { session: s, .. }
            | CaptureState::Enrolling { session: s }
            | CaptureState::Meeting { session: s } => s == session,
            CaptureState::Idle => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CaptureState::*;
    use Request::*;

    #[test]
    fn idle_start_begins_dictation_with_the_next_session_id() {
        let (state, t) = transition(Idle, StartDictation(Trigger::Hotkey), 7);
        assert_eq!(
            state,
            Dictating {
                trigger: Trigger::Hotkey,
                session: 7
            }
        );
        assert_eq!(t, Transition::StartedDictation { session: 7 });
    }

    #[test]
    fn start_while_dictating_is_rejected_not_toggled_off() {
        let current = Dictating {
            trigger: Trigger::Hotkey,
            session: 1,
        };
        let (state, t) = transition(current, StartDictation(Trigger::DeepLink), 2);
        assert_eq!(state, current);
        assert!(matches!(t, Transition::Rejected(_)));
    }

    #[test]
    fn discard_ends_a_dictation_and_is_rejected_otherwise() {
        let current = Dictating {
            trigger: Trigger::Hotkey,
            session: 4,
        };
        assert_eq!(
            transition(current, DiscardDictation, 5),
            (Idle, Transition::DiscardedDictation { session: 4 })
        );
        assert!(matches!(
            transition(Idle, DiscardDictation, 5),
            (Idle, Transition::Rejected(_))
        ));
        let enrolling = Enrolling { session: 2 };
        assert!(matches!(
            transition(enrolling, DiscardDictation, 5),
            (Enrolling { session: 2 }, Transition::Rejected(_))
        ));
    }

    #[test]
    fn stop_while_idle_is_rejected() {
        let (state, t) = transition(Idle, StopDictation(Trigger::DeepLink), 1);
        assert_eq!(state, Idle);
        assert!(matches!(t, Transition::Rejected(_)));
    }

    #[test]
    fn toggle_flips_both_ways() {
        let (state, t) = transition(Idle, ToggleDictation(Trigger::Button), 3);
        assert_eq!(t, Transition::StartedDictation { session: 3 });
        let (state, t) = transition(state, ToggleDictation(Trigger::Hotkey), 4);
        assert_eq!(state, Idle);
        assert_eq!(t, Transition::StoppedDictation { session: 3 });
    }

    #[test]
    fn any_trigger_can_stop_a_dictation_started_by_another() {
        let current = Dictating {
            trigger: Trigger::DoubleTapFn,
            session: 5,
        };
        let (state, t) = transition(current, ToggleDictation(Trigger::Hotkey), 6);
        assert_eq!(state, Idle);
        assert_eq!(t, Transition::StoppedDictation { session: 5 });
    }

    #[test]
    fn enrollment_and_dictation_exclude_each_other() {
        let enrolling = Enrolling { session: 1 };
        for req in [
            StartDictation(Trigger::Hotkey),
            ToggleDictation(Trigger::Hotkey),
            StopDictation(Trigger::Hotkey),
        ] {
            let (state, t) = transition(enrolling, req, 2);
            assert_eq!(state, enrolling);
            assert!(matches!(t, Transition::Rejected(_)));
        }

        let dictating = Dictating {
            trigger: Trigger::Hotkey,
            session: 1,
        };
        for req in [StartEnrollment, StopEnrollment] {
            let (state, t) = transition(dictating, req, 2);
            assert_eq!(state, dictating);
            assert!(matches!(t, Transition::Rejected(_)));
        }
    }

    #[test]
    fn enrollment_round_trip() {
        let (state, t) = transition(Idle, StartEnrollment, 9);
        assert_eq!(t, Transition::StartedEnrollment { session: 9 });
        let (state, t) = transition(state, StopEnrollment, 10);
        assert_eq!(state, Idle);
        assert_eq!(t, Transition::StoppedEnrollment { session: 9 });
    }

    #[test]
    fn meeting_round_trip() {
        let (state, t) = transition(Idle, StartMeeting, 4);
        assert_eq!(t, Transition::StartedMeeting { session: 4 });
        let (state, t) = transition(state, StopMeeting, 5);
        assert_eq!(state, Idle);
        assert_eq!(t, Transition::StoppedMeeting { session: 4 });
    }

    #[test]
    fn a_meeting_holds_the_mic_until_it_ends() {
        let meeting = Meeting { session: 2 };
        for request in [
            StartDictation(Trigger::Hotkey),
            ToggleDictation(Trigger::Button),
            StopDictation(Trigger::Tray),
            DiscardDictation,
            StartEnrollment,
            StartMeeting,
        ] {
            let (state, t) = transition(meeting, request, 3);
            assert_eq!(state, meeting, "{request:?}");
            assert!(matches!(t, Transition::Rejected(_)), "{request:?}");
        }
        let dictating = Dictating {
            trigger: Trigger::Hotkey,
            session: 1,
        };
        let (state, t) = transition(dictating, StartMeeting, 2);
        assert_eq!(state, dictating);
        assert!(matches!(t, Transition::Rejected(_)));
        assert!(matches!(
            transition(Idle, StopMeeting, 1).1,
            Transition::Rejected(_)
        ));
    }

    #[test]
    fn controller_issues_fresh_session_ids_and_tracks_the_active_one() {
        let c = CaptureController::new();
        let Transition::StartedDictation { session: first } =
            c.request(ToggleDictation(Trigger::Hotkey))
        else {
            panic!("expected start");
        };
        assert!(c.is_session_active(first));
        c.request(ToggleDictation(Trigger::Hotkey));
        assert!(!c.is_session_active(first));

        let Transition::StartedDictation { session: second } =
            c.request(ToggleDictation(Trigger::Hotkey))
        else {
            panic!("expected start");
        };
        assert_ne!(first, second);
        assert!(
            !c.is_session_active(first),
            "a stale session id must not read as active"
        );
        assert!(c.is_session_active(second));
    }

    #[test]
    fn rejected_requests_do_not_consume_a_session_id() {
        let c = CaptureController::new();
        c.request(StopDictation(Trigger::DeepLink));
        assert_eq!(
            c.request(StartDictation(Trigger::Hotkey)),
            Transition::StartedDictation { session: 1 }
        );
    }
}
