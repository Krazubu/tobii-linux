//! Power save in and out: commands 3000 and 3010, as the DLL's
//! `tracker_power_save_activate` and `tracker_power_save_deactivate`
//! (0x18019d730, 0x18019fbc0) send them for PRP property 2. Neither carries a
//! payload, and the answer carries none either: all it reports is the error
//! word.
//!
//! The ET5 carries both out, and announces each change with a notification,
//! 3020 when power save begins and 3030 when it ends (probe 2026-10-21). It is
//! those notifications, folded into the facts, that the `POWER_SAVE_ACTIVE`
//! state answers from (see [`crate::requests`]), not the request here: the
//! DLL's setter writes no cache, and only the notifications and a 1490 do.
//! Power save does not stop the sensor: gaze kept coming at full rate with it
//! on.
//!
//! Power save is one state for the device, as in the DLL: the last request
//! wins and any client may end it. The DLL has no notion of a client going
//! away, since it lives in one process; the daemon does, and when the client
//! whose request is in effect goes away it ends power save for it, rather
//! than leave the tracker in a mode nobody is left to know about. That holder
//! is kept across a re-initialisation of the device, unlike the pause's: the
//! pause is undone by every init replay, but whether power save survives a
//! re-open was never tested, so a departing holder always sends its 3010 —
//! which a tracker already out of power save answers `0` (probe 2026-10-21).
//! The daemon never re-applies power save after a re-init, as the DLL never
//! does (each setter has one caller, and create and reconnect only read the
//! state back from the 1490).
//!
//! Requests hold `power_save_lock` from before the command to the holder's
//! update, so the device and the holder agree; the state lock is taken only
//! briefly and never across the command.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tobii_ipc::deadline;
use tobii_ipc::request::status;
use tobii_proto::protocol::cmd;
use tracing::{info, warn};

use crate::daemon::{State, lock_state, lock_until, look_for_tracker};
use crate::device::run;
use crate::requests::Reply;

/// How long the device may take to answer 3000 or 3010.
const POWER_SAVE_TIMEOUT: Duration = deadline::POWER_SAVE;
/// How long a request waits for another power-save request to finish.
#[cfg(not(test))]
const LOCK_TIMEOUT: Duration = deadline::POWER_SAVE_LOCK;
#[cfg(test)]
const LOCK_TIMEOUT: Duration = Duration::from_millis(300);

/// Answer a request to enter (`[1]`) or leave (`[0]`) power save from
/// `client`.
pub(crate) fn handle(state: &Mutex<State>, client: u64, payload: &[u8]) -> Reply {
    match payload {
        [1] => set(state, client, true),
        [0] => set(state, client, false),
        _ => Reply::err(status::INVALID_PARAMETER),
    }
}

/// The power-save lock, as the state holds it.
fn power_save_lock(state: &Mutex<State>) -> Arc<Mutex<()>> {
    Arc::clone(&lock_state(state).power_save_lock)
}

/// The command that enters or leaves power save.
const fn command_for(on: bool) -> u32 {
    if on {
        cmd::POWER_SAVE_ACTIVATE
    } else {
        cmd::POWER_SAVE_DEACTIVATE
    }
}

fn set(state: &Mutex<State>, client: u64, on: bool) -> Reply {
    let deadline = Instant::now() + LOCK_TIMEOUT;
    let lock = power_save_lock(state);
    let Some(_held) = lock_until(&lock, deadline) else {
        return Reply::err(status::TIMED_OUT);
    };
    let on_bus = look_for_tracker(state);
    let Some(device) = lock_state(state).device_for(client, on_bus) else {
        return Reply::err(status::CONNECTION_FAILED);
    };
    match run(
        device.as_ref(),
        command_for(on),
        Vec::new(),
        POWER_SAVE_TIMEOUT,
    ) {
        Ok(_) => {
            info!(client, on, "power save set");
            lock_state(state).power_save_holder = on.then_some(client);
            Reply::ok(Vec::new())
        }
        Err(code) => Reply::err(code),
    }
}

/// `client` went away: if its request to enter power save is in effect, end
/// power save. Its reader calls this before it unlists the client. Only the
/// running engine is asked; none is started for this. A 3010 answered 0
/// leaves the facts saying power save is off at once (see
/// [`State::note_power_save_ended`]): the engine may stop right after, before
/// the tracker's 3030 is read (probe 2026-10-21, where a client asking a
/// second later was told it was still on).
pub(crate) fn release(state: &Mutex<State>, client: u64) {
    // No request of the client's runs any more (its reader waits for its
    // request worker before calling this), so the holder cannot become
    // `client` after this check.
    if lock_state(state).power_save_holder != Some(client) {
        return;
    }
    let deadline = Instant::now() + LOCK_TIMEOUT;
    let lock = power_save_lock(state);
    let Some(_held) = lock_until(&lock, deadline) else {
        // Another request has held the lock all this while, stuck on the
        // device; a 3010 now could cross it. The departed client is not left
        // holding power save, though nothing ends it.
        warn!(
            client,
            "client in power save disconnected; could not end it in time, so it may stay on"
        );
        let mut st = lock_state(state);
        if st.power_save_holder == Some(client) {
            st.power_save_holder = None;
        }
        return;
    };
    let device = {
        let mut st = lock_state(state);
        if st.power_save_holder != Some(client) {
            return;
        }
        st.power_save_holder = None;
        st.commands()
    };
    let Some(device) = device else {
        warn!(
            client,
            "client in power save disconnected with no engine running; it may stay on"
        );
        return;
    };
    info!(
        client,
        "client in power save disconnected; ending power save"
    );
    match run(
        device.as_ref(),
        command_for(false),
        Vec::new(),
        POWER_SAVE_TIMEOUT,
    ) {
        Ok(_) => {
            let state_updated = lock_state(state).note_power_save_ended();
            info!(state_updated, "power save ended for the departed client");
        }
        Err(code) => warn!(status = code, "could not end power save"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::tests::state_with_client;
    use crate::device::DeviceCommands;
    use tobii_ipc::request::{Request, kind};
    use tobii_proto::facts::DeviceFacts;
    use tobii_proto::protocol::ttp_error;
    use tobii_usb::engine::{CommandError, CommandResponse, Sample};

    /// Records every command and answers each with `answer`.
    struct Recording {
        sent: Mutex<Vec<(u32, Vec<u8>)>>,
        deadlines: Mutex<Vec<Duration>>,
        answer: Mutex<Result<CommandResponse, CommandError>>,
    }

    impl DeviceCommands for Recording {
        fn run(
            &self,
            cmd: u32,
            payload: Vec<u8>,
            timeout: Duration,
        ) -> Result<CommandResponse, CommandError> {
            self.sent.lock().expect("sent").push((cmd, payload));
            self.deadlines.lock().expect("deadlines").push(timeout);
            self.answer.lock().expect("answer").clone()
        }
    }

    struct Setup {
        state: Mutex<State>,
        device: Arc<Recording>,
    }

    /// Clients 1 and 2, and a stand-in device that answers, so no engine
    /// starts.
    fn setup() -> Setup {
        let device = Arc::new(Recording {
            sent: Mutex::new(Vec::new()),
            deadlines: Mutex::new(Vec::new()),
            answer: Mutex::new(Ok(CommandResponse::ok(Vec::new()))),
        });
        let mut st = state_with_client(1);
        st.clients.append(&mut state_with_client(2).clients);
        st.fake_device = Some(device.clone());
        Setup {
            state: Mutex::new(st),
            device,
        }
    }

    /// A request from `client`, through the daemon's dispatcher.
    fn ask(s: &Setup, client: u64, payload: &[u8]) -> Reply {
        crate::requests::handle(
            &s.state,
            client,
            &Request {
                id: 1,
                kind: kind::POWER_SAVE,
                payload,
            },
        )
    }

    fn sent(s: &Setup) -> Vec<(u32, Vec<u8>)> {
        std::mem::take(&mut *s.device.sent.lock().expect("sent"))
    }

    fn holder(s: &Setup) -> Option<u64> {
        lock_state(&s.state).power_save_holder
    }

    #[test]
    fn entering_and_leaving_send_3000_and_3010_with_no_payload() {
        let s = setup();
        assert_eq!(ask(&s, 1, &[1]), Reply::ok(Vec::new()));
        assert_eq!(sent(&s), [(3000, Vec::new())]);
        assert_eq!(holder(&s), Some(1));
        assert_eq!(ask(&s, 1, &[0]), Reply::ok(Vec::new()));
        assert_eq!(sent(&s), [(3010, Vec::new())]);
        assert_eq!(holder(&s), None);
        assert!(
            s.device
                .deadlines
                .lock()
                .expect("deadlines")
                .iter()
                .all(|d| *d == deadline::POWER_SAVE),
            "every command waits the power-save deadline"
        );
    }

    #[test]
    fn a_request_waits_on_the_device_within_its_budget() {
        let s = setup();
        for payload in [[1], [0]] {
            s.device.deadlines.lock().expect("deadlines").clear();
            assert_eq!(ask(&s, 1, &payload), Reply::ok(Vec::new()));
            let sent = s.device.deadlines.lock().expect("deadlines").clone();
            // After waiting for another power-save request to finish.
            let waited = deadline::POWER_SAVE_LOCK + deadline::commands(&sent);
            assert!(
                !sent.is_empty() && waited <= deadline::worst::POWER_SAVE,
                "{payload:?}: {waited:?}"
            );
        }
    }

    #[test]
    fn the_last_request_wins_and_any_client_may_leave() {
        let s = setup();
        ask(&s, 1, &[1]);
        assert_eq!(holder(&s), Some(1));
        ask(&s, 2, &[1]);
        assert_eq!(holder(&s), Some(2), "the last request wins");
        ask(&s, 1, &[0]);
        assert_eq!(holder(&s), None, "any client may leave power save");
    }

    #[test]
    fn a_departing_holder_ends_power_save() {
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        release(&s.state, 1);
        assert_eq!(sent(&s), [(3010, Vec::new())]);
        assert_eq!(holder(&s), None);
    }

    /// Facts whose 1490 reported power save as `value`.
    fn facts_saying(value: &str) -> Arc<DeviceFacts> {
        Arc::new(DeviceFacts {
            status: vec![(tobii_proto::facts::STATUS_POWER_SAVE, value.to_owned())],
            ..DeviceFacts::default()
        })
    }

    fn power_save_in_facts(s: &Setup) -> Option<String> {
        lock_state(&s.state).facts.as_ref().and_then(|f| {
            f.status_string(tobii_proto::facts::STATUS_POWER_SAVE)
                .map(str::to_owned)
        })
    }

    #[test]
    fn a_departing_holder_leaves_the_facts_saying_power_save_is_off() {
        // The engine may stop before the tracker's 3030 is read.
        let s = setup();
        ask(&s, 1, &[1]);
        lock_state(&s.state).facts = Some(facts_saying("true"));
        release(&s.state, 1);
        assert_eq!(power_save_in_facts(&s).as_deref(), Some("false"));
    }

    #[test]
    fn a_refused_3010_leaves_the_facts_as_they_were() {
        let s = setup();
        ask(&s, 1, &[1]);
        lock_state(&s.state).facts = Some(facts_saying("true"));
        *s.device.answer.lock().expect("answer") = Ok(CommandResponse {
            status: 1,
            error: ttp_error::NOT_SUPPORTED,
            payload: Vec::new(),
        });
        release(&s.state, 1);
        assert_eq!(power_save_in_facts(&s).as_deref(), Some("true"));
    }

    #[test]
    fn a_departing_client_that_does_not_hold_it_sends_nothing() {
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        release(&s.state, 2);
        assert!(sent(&s).is_empty());
        assert_eq!(holder(&s), Some(1));
    }

    #[test]
    fn a_departing_holder_that_cannot_get_the_lock_is_forgotten() {
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        let lock = power_save_lock(&s.state);
        let held = lock.lock().expect("power-save lock");
        release(&s.state, 1);
        drop(held);
        assert!(sent(&s).is_empty(), "no 3010 without the lock");
        assert_eq!(holder(&s), None, "the departed client holds nothing");
    }

    #[test]
    fn the_holder_outlives_a_reinit_so_its_departure_still_ends_power_save() {
        // Whether power save survives a re-open was never tested; a 3010 to a
        // tracker already out of it is answered 0, so it is always sent.
        let s = setup();
        ask(&s, 1, &[1]);
        sent(&s);
        lock_state(&s.state).observe(&Sample::DeviceReady(Arc::new(DeviceFacts::default())));
        assert_eq!(holder(&s), Some(1));
        release(&s.state, 1);
        assert_eq!(sent(&s), [(3010, Vec::new())]);
    }

    #[test]
    fn a_refusal_is_passed_on_and_holds_nothing() {
        let s = setup();
        *s.device.answer.lock().expect("answer") = Ok(CommandResponse {
            status: 1,
            error: ttp_error::NOT_SUPPORTED,
            payload: Vec::new(),
        });
        assert_eq!(ask(&s, 1, &[1]), Reply::err(status::NOT_SUPPORTED));
        assert_eq!(holder(&s), None);
    }

    #[test]
    fn a_payload_that_is_neither_is_refused_and_sends_nothing() {
        let s = setup();
        for payload in [&[][..], &[2], &[1, 0]] {
            assert_eq!(ask(&s, 1, payload), Reply::err(status::INVALID_PARAMETER));
        }
        assert!(sent(&s).is_empty());
    }
}
