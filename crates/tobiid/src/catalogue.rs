//! The tracker's catalogue streams that clients hold: 0x1770, the Stream
//! Engine's custom stream 0 (`tobii_custom_stream_subscribe`), and 0x508,
//! image collection (`tobii_image_collection_subscribe`), the two streams of
//! the tracker's catalogue (command 1200) that the DLL starts for a client
//! that subscribes to one, with command 1220 from its tracker module
//! (0x180160810, 0x180160210), and stops with 1230. A client holds one with
//! `CATALOGUE_STREAM_START` and lets it go with `CATALOGUE_STREAM_STOP` or by
//! going away (see [`tobii_ipc::request::kind`]); the engine sends the
//! commands ([`Commands::change_stream`](tobii_usb::engine::Commands::change_stream)).
//!
//! The tracker runs a stream while a client holds it. The first to start
//! one has its 1220 sent and is answered what came of it, the tracker's
//! result as the DLL gives one (see [`crate::device::stream_result`]): it
//! holds the stream only if the tracker took the start. A client that starts
//! a stream another holds joins it, with nothing sent, and the last to let
//! one go has its 1230 sent. The engine starts what it runs again after each
//! of its re-opens, and an engine started in place of another is seeded with
//! the streams held then, so nothing here sends a command for either.
//! Holding a stream keeps the device up for the client, as a request does
//! ([`State::device_for`]).
//!
//! A start that may have reached the tracker without its answer coming back
//! (none in time, or the engine's open ended meanwhile) is followed by a
//! 1230, not waited for, unless a client holds the stream by then: should
//! the tracker have taken the start, the engine would otherwise keep running
//! a stream nobody holds.
//!
//! A client that reconnects restores what it held
//! ([`STREAM_START_RESTORE`](tobii_ipc::request::STREAM_START_RESTORE)): it
//! holds the stream at once, with no look at the bus and no wait for the
//! facts, and the stream is started once the reply is queued, if nobody held
//! it before ([`FollowUp`]), on a thread of its own: the client's next
//! request, the restore of its other stream among them, does not wait for
//! that start. The client holds the stream whatever the tracker answers.
//!
//! Only these two streams are started or stopped here, and only while the
//! tracker's catalogue lists them: never the gaze and presence streams
//! (0x500, 0x504), which the init starts and whose 1230 would stop gaze, nor
//! the image stream the engine runs itself (0x50e).
//!
//! The custom stream's frames go to the clients that hold it, whatever
//! their SUBSCRIBE masks, under a routing bit of the daemon's own
//! ([`ROUTE_CUSTOM`]); a subscription changes nothing a client holds.
//!
//! # Locking
//!
//! Who holds what changes only under the state lock. Each decision that a
//! stream gets its first holder or loses its last is made, and the 1220 or
//! 1230 that goes with it sent, while `catalogue_lock` is held: by a start,
//! a stop, a client going away, and the start a restore leaves to run after
//! its reply. A restore registers its holder under the state lock alone, and
//! sends nothing; the start it leaves runs off the client's request worker,
//! so that the lock alone orders it against the client's later stop or its
//! going away, and it starts the stream if some client holds it by then.
//! `catalogue_lock` is taken before the state lock, never under it, and
//! never with `pause_lock` or `name_lock` held; the state lock is never held
//! across a device command. So a client may join a stream under the state
//! lock alone: a stream is held only once the tracker took its start, or a
//! restore claimed it (its start follows under the lock), and it loses its
//! last holder only while `catalogue_lock` is held until its 1230 is
//! answered, so that a start coming in meanwhile, which finds the stream
//! unheld, sends its 1220 after that 1230.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tobii_ipc::deadline;
use tobii_ipc::request::{
    StreamStart, decode_stream_start, decode_u32, encode_u32, status, tracker_error,
};
use tobii_proto::facts::DeviceFacts;
use tobii_proto::protocol::{STREAM_ID_ALGODBG, STREAM_ID_IMAGE_COLLECTION};
use tobii_usb::engine::StreamChange;
use tracing::{debug, info, warn};

use crate::daemon::{State, lock_state, lock_until, look_for_tracker};
use crate::device::stream_result;
use crate::requests::{FollowUp, Reply, wait_for_facts};

/// The streams a client may hold: the custom stream, 0x1770, and image
/// collection, 0x508, the two the DLL starts for a subscriber.
pub(crate) const STARTABLE: [u32; 2] = [STREAM_ID_ALGODBG, STREAM_ID_IMAGE_COLLECTION];

/// How long the tracker may take to answer a stream's 1220 or 1230: as long
/// as the DLL's tracker module waits for either.
const STREAM_TIMEOUT: Duration = deadline::STREAM;
/// How long a start or stop waits for another start or stop of a catalogue
/// stream to finish. A client going away, and the start a restore leaves,
/// wait on past it, warning once.
#[cfg(not(test))]
const LOCK_TIMEOUT: Duration = deadline::STREAM_LOCK;
#[cfg(test)]
const LOCK_TIMEOUT: Duration = Duration::from_millis(300);

/// The daemon's own stream bit for the custom stream's frames
/// ([`tobii_ipc::TAG_CUSTOM_STREAM`]), past every bit of the protocol. A
/// client is written the frames of its SUBSCRIBE mask and of this bit while
/// it holds the stream ([`Holders::routes_of`]), and no SUBSCRIBE sets it
/// (the daemon clears it in one), so that the frames reach the clients that
/// hold the stream and no other.
pub(crate) const ROUTE_CUSTOM: u32 = 1 << 31;

/// The routing bit of the frames of catalogue stream `stream`:
/// [`ROUTE_CUSTOM`] for the custom stream, and none (0) for any other.
/// Nothing sends image collection's frames: the ET5 refuses to start the
/// stream, so none was ever seen to lay one out from.
#[must_use]
pub(crate) const fn route(stream: u32) -> u32 {
    if stream == STREAM_ID_ALGODBG {
        ROUTE_CUSTOM
    } else {
        0
    }
}

/// What [`Holders::remove`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Removed {
    /// The client did not hold the stream.
    NotHeld,
    /// Another client holds it still.
    Others,
    /// The client was the last to hold it.
    Last,
}

/// Which clients hold which catalogue stream: for each stream held, in id
/// order, the clients that hold it, each once however many times it started
/// the stream. A stream nobody holds has no entry.
#[derive(Debug, Default)]
pub(crate) struct Holders {
    by_stream: Vec<(u32, Vec<u64>)>,
}

impl Holders {
    /// The clients that hold `stream`: none for a stream nobody holds.
    fn clients(&self, stream: u32) -> &[u64] {
        self.by_stream
            .iter()
            .find(|(id, _)| *id == stream)
            .map_or(&[], |(_, clients)| clients.as_slice())
    }

    /// Whether `client` holds `stream`.
    #[must_use]
    pub(crate) fn holds(&self, stream: u32, client: u64) -> bool {
        self.clients(stream).contains(&client)
    }

    /// Whether some client holds `stream`.
    #[must_use]
    pub(crate) fn is_held(&self, stream: u32) -> bool {
        !self.clients(stream).is_empty()
    }

    /// Whether `client` holds any stream.
    #[must_use]
    pub(crate) fn holds_any(&self, client: u64) -> bool {
        self.by_stream
            .iter()
            .any(|(_, clients)| clients.contains(&client))
    }

    /// `client` holds `stream`, if it did not already.
    pub(crate) fn add(&mut self, stream: u32, client: u64) {
        let at = self.by_stream.partition_point(|(id, _)| *id < stream);
        match self.by_stream.get_mut(at) {
            Some((id, clients)) if *id == stream => {
                if !clients.contains(&client) {
                    clients.push(client);
                }
            }
            _ => self.by_stream.insert(at, (stream, vec![client])),
        }
    }

    /// `client` holds `stream` no longer: whether it did, and whether
    /// another client holds it still.
    #[must_use]
    pub(crate) fn remove(&mut self, stream: u32, client: u64) -> Removed {
        let Some(at) = self.by_stream.iter().position(|(id, _)| *id == stream) else {
            return Removed::NotHeld;
        };
        let clients = &mut self.by_stream[at].1;
        let before = clients.len();
        clients.retain(|&c| c != client);
        if clients.len() == before {
            Removed::NotHeld
        } else if clients.is_empty() {
            self.by_stream.remove(at);
            Removed::Last
        } else {
            Removed::Others
        }
    }

    /// `client` holds nothing any more: the streams it was the last to
    /// hold, in id order.
    #[must_use]
    pub(crate) fn release(&mut self, client: u64) -> Vec<u32> {
        let mut emptied = Vec::new();
        self.by_stream.retain_mut(|(id, clients)| {
            let before = clients.len();
            clients.retain(|&c| c != client);
            let last = clients.is_empty() && before > 0;
            if last {
                emptied.push(*id);
            }
            !last
        });
        emptied
    }

    /// The streams some client holds, in id order: those an engine started
    /// now is to run.
    #[must_use]
    pub(crate) fn held(&self) -> Vec<u32> {
        self.by_stream.iter().map(|(id, _)| *id).collect()
    }

    /// The routing bits of the frames `client` gets for the streams it
    /// holds (see [`route`]).
    #[must_use]
    pub(crate) fn routes_of(&self, client: u64) -> u32 {
        self.by_stream
            .iter()
            .filter(|(_, clients)| clients.contains(&client))
            .fold(0, |routes, (id, _)| routes | route(*id))
    }
}

/// The catalogue lock, as the state holds it.
fn catalogue_lock(state: &Mutex<State>) -> Arc<Mutex<()>> {
    Arc::clone(&lock_state(state).catalogue_lock)
}

/// Whether the tracker's catalogue, as `facts` give it, lists `stream`:
/// never when they give none (every init of the daemon's lost its 1200,
/// as facts keep the last catalogue an init got), and an engine then
/// starts no catalogue stream either, as its catalogue comes from those
/// inits too.
fn lists(facts: &DeviceFacts, stream: u32) -> bool {
    facts.streams.iter().any(|s| s.id == stream)
}

/// The `OK` reply that carries tracker result `result`
/// ([`tracker_error`]).
fn reply_with(result: u32) -> Reply {
    Reply::ok(encode_u32(result))
}

/// Have `client` join `stream` if some client holds it (itself among
/// them), keeping the device up for it as a start does: whether it holds
/// the stream now. Nothing is sent: the stream runs.
fn join(st: &mut State, client: u64, stream: u32) -> bool {
    if !st.catalogue.is_held(stream) {
        return false;
    }
    st.pin_device(client);
    st.catalogue.add(stream, client);
    debug!(
        client,
        stream = %format_args!("{stream:#x}"),
        "a client joined a catalogue stream already held"
    );
    true
}

/// Answer a `CATALOGUE_STREAM_START` from `client`: payload `u32` stream id,
/// then optional flags (see [`decode_stream_start`]), a restore among them
/// (see [`restore`]).
///
/// A stream some client holds is joined, under the state lock alone, with
/// nothing sent. Otherwise the start needs a tracker: `CONNECTION_FAILED`
/// at once with none on the bus, with neither the facts nor the lock waited
/// for; the client keeps the device up all the same, so that the watchdog
/// starts an engine for it once the tracker is plugged in. Then the facts,
/// for the catalogue (`TIMED_OUT` without them in time, `NOT_SUPPORTED` for
/// a stream the catalogue does not list), and the lock (`TIMED_OUT` when
/// another start or stop takes too long); a stream a start or a restore
/// made held meanwhile is joined. Then the 1220, whose result the reply
/// carries: the client holds the stream if the tracker took it and the
/// engine it went to still runs. An engine started in place of a lost one
/// was seeded without the stream, so a start the lost one took is
/// `CONNECTION_FAILED`. A start the engine does not make, of a stream its
/// catalogue does not list, is `NOT_SUPPORTED`.
pub(crate) fn start(state: &Mutex<State>, client: u64, payload: &[u8]) -> Reply {
    let Some(StreamStart {
        stream_id: stream,
        restore: restoring,
    }) = decode_stream_start(payload)
    else {
        return Reply::err(status::INVALID_PARAMETER);
    };
    if !STARTABLE.contains(&stream) {
        return Reply::err(status::NOT_SUPPORTED);
    }
    if restoring {
        return restore(state, client, stream);
    }
    if join(&mut lock_state(state), client, stream) {
        return reply_with(tracker_error::NO_ERROR);
    }
    let on_bus = look_for_tracker(state);
    if lock_state(state).device_for(client, on_bus).is_none() {
        return Reply::err(status::CONNECTION_FAILED);
    }
    match wait_for_facts(state) {
        Ok(facts) if lists(&facts, stream) => {}
        Ok(_) => return Reply::err(status::NOT_SUPPORTED),
        Err(code) => return Reply::err(code),
    }
    let lock = catalogue_lock(state);
    let Some(_held) = lock_until(&lock, Instant::now() + LOCK_TIMEOUT) else {
        return Reply::err(status::TIMED_OUT);
    };
    let on_bus = look_for_tracker(state);
    let (device, losses) = {
        let mut st = lock_state(state);
        if join(&mut st, client, stream) {
            return reply_with(tracker_error::NO_ERROR);
        }
        let Some(device) = st.device_for(client, on_bus) else {
            return Reply::err(status::CONNECTION_FAILED);
        };
        (device, st.engine_losses)
    };
    let change = StreamChange::Start(stream);
    let result = match stream_result(change, device.change_stream(change, STREAM_TIMEOUT).wait()) {
        Ok(result) => result,
        Err(code) => return Reply::err(code),
    };
    let (result, undo) = settle(&mut lock_state(state), client, stream, result, losses);
    if undo {
        // Queued behind the 1220 while the lock is held: a start of the
        // stream queued after this one comes after it.
        drop(device.change_stream(StreamChange::Stop(stream), STREAM_TIMEOUT));
    }
    reply_with(result)
}

/// What a start of `stream` from `client` comes to, the tracker having
/// answered `result` while `losses` engines were lost before it was sent:
/// the client holds the stream if the tracker took it and no engine was
/// lost since. With it, whether the stream is to be stopped again: when
/// the start may have reached the tracker without its answer coming back
/// (none in time, or the engine's open ended meanwhile) and nobody holds the
/// stream now. A start the lost engine took needs no stop: that engine
/// stopped what it ran as it ended.
fn settle(st: &mut State, client: u64, stream: u32, result: u32, losses: u64) -> (u32, bool) {
    let same_engine = st.engine_losses == losses;
    if result == tracker_error::NO_ERROR {
        if same_engine {
            st.catalogue.add(stream, client);
            info!(
                client,
                stream = %format_args!("{stream:#x}"),
                "catalogue stream started"
            );
            return (result, false);
        }
        warn!(
            client,
            stream = %format_args!("{stream:#x}"),
            "the engine was lost as the tracker started a catalogue stream; nobody holds it"
        );
        return (tracker_error::CONNECTION_FAILED, false);
    }
    let unanswered = result == tracker_error::FIRMWARE_NO_RESPONSE
        || (result == tracker_error::CONNECTION_FAILED && same_engine);
    let undo = unanswered && !st.catalogue.is_held(stream);
    if undo {
        warn!(
            client,
            stream = %format_args!("{stream:#x}"),
            result,
            "a catalogue stream's start got no answer; stopping the stream in case the tracker started it"
        );
    }
    (result, undo)
}

/// A start that restores a claim, what libtobii sends for each stream it
/// held when it reconnects: `client` holds `stream` at once, under the state
/// lock alone, with no look at the bus and no wait for the facts, and keeps
/// the device up, so that the watchdog starts an engine if none runs.
/// Refused (`NOT_SUPPORTED`), once the facts are known, for a stream their
/// catalogue does not list, as a start is (facts without a catalogue list
/// none: no engine would start the stream), so that the reconnect fails, as
/// the DLL's fails when its module refuses to subscribe a custom stream
/// again, its catalogue lacking it (0x1801608b5, then 0x18015a0be). A stream
/// some client holds is joined; one nobody held is started once the reply
/// is queued ([`start_restored`]).
fn restore(state: &Mutex<State>, client: u64, stream: u32) -> Reply {
    let mut st = lock_state(state);
    if st.facts.as_deref().is_some_and(|f| !lists(f, stream)) {
        return Reply::err(status::NOT_SUPPORTED);
    }
    if join(&mut st, client, stream) {
        return reply_with(tracker_error::NO_ERROR);
    }
    st.pin_device(client);
    st.catalogue.add(stream, client);
    info!(
        client,
        stream = %format_args!("{stream:#x}"),
        "a client restored its claim on a catalogue stream"
    );
    Reply {
        then: Some(FollowUp::StartRestored { client, stream }),
        ..reply_with(tracker_error::NO_ERROR)
    }
}

/// Start `stream`, which a restore had `client` hold when nobody held it:
/// what the restore's reply leaves to run once it is queued, on a thread of
/// its own (see `daemon::run_requests`), so that the client's next request
/// does not wait for it. It waits for the lock as long as it takes, as a
/// client going away does ([`release`]), warning once past the time a
/// request waits for it: given up on, the claim would stand with the stream
/// started neither by the engine that runs nor at its re-opens. Then a 1220
/// through the engine that runs, if some client holds the stream still: the
/// client, or one that joined the stream meanwhile and counts on this start
/// though the client let go. With no engine running, nothing: the next
/// engine is seeded with the stream. The stream stays held whatever the
/// tracker answers: a start not taken is warned of. A start of a stream
/// that runs already (a client's start took the lock first) is answered by
/// the engine with nothing written.
pub(crate) fn start_restored(state: &Mutex<State>, client: u64, stream: u32) {
    let lock = catalogue_lock(state);
    let _held = lock_until(&lock, Instant::now() + LOCK_TIMEOUT).unwrap_or_else(|| {
        warn!(
            client,
            stream = %format_args!("{stream:#x}"),
            "a restored catalogue stream waits for another start or stop to finish before it is started"
        );
        lock.lock().unwrap_or_else(PoisonError::into_inner)
    });
    let device = {
        let st = lock_state(state);
        if !st.catalogue.is_held(stream) {
            return;
        }
        st.commands()
    };
    let Some(device) = device else {
        debug!(
            client,
            stream = %format_args!("{stream:#x}"),
            "no engine runs to start a restored catalogue stream; the next one starts it"
        );
        return;
    };
    let change = StreamChange::Start(stream);
    match stream_result(change, device.change_stream(change, STREAM_TIMEOUT).wait()) {
        Ok(tracker_error::NO_ERROR) => info!(
            client,
            stream = %format_args!("{stream:#x}"),
            "restored catalogue stream started"
        ),
        Ok(result) => warn!(
            client,
            stream = %format_args!("{stream:#x}"),
            result,
            "a restored catalogue stream did not start; it stays held all the same"
        ),
        Err(code) => warn!(
            client,
            stream = %format_args!("{stream:#x}"),
            status = code,
            "a restored catalogue stream was not started; it stays held all the same"
        ),
    }
}

/// Answer a `CATALOGUE_STREAM_STOP` from `client`: payload `u32` stream id
/// (a tail is ignored). It needs neither the facts nor a tracker, and
/// starts no engine. A client that does not hold the stream is answered at
/// once, with nothing sent; one that does holds it no longer once the lock
/// is taken (`TIMED_OUT`, holding it still, when another start or stop takes
/// too long). The last to hold it has its 1230 sent through the engine that
/// runs, and the reply carries what came of it. With no engine running, or
/// one whose catalogue does not list the stream, nothing runs the stream:
/// the reply is `NO_ERROR`, and the next engine is seeded without it.
pub(crate) fn stop(state: &Mutex<State>, client: u64, payload: &[u8]) -> Reply {
    let Some(stream) = decode_u32(payload) else {
        return Reply::err(status::INVALID_PARAMETER);
    };
    if !STARTABLE.contains(&stream) {
        return Reply::err(status::NOT_SUPPORTED);
    }
    if !lock_state(state).catalogue.holds(stream, client) {
        return reply_with(tracker_error::NO_ERROR);
    }
    let lock = catalogue_lock(state);
    let Some(_held) = lock_until(&lock, Instant::now() + LOCK_TIMEOUT) else {
        return Reply::err(status::TIMED_OUT);
    };
    let device = {
        let mut st = lock_state(state);
        match st.catalogue.remove(stream, client) {
            Removed::NotHeld | Removed::Others => return reply_with(tracker_error::NO_ERROR),
            Removed::Last => st.commands(),
        }
    };
    let Some(device) = device else {
        return reply_with(tracker_error::NO_ERROR);
    };
    let change = StreamChange::Stop(stream);
    let result = stream_result(change, device.change_stream(change, STREAM_TIMEOUT).wait())
        .unwrap_or(tracker_error::NO_ERROR);
    info!(
        client,
        stream = %format_args!("{stream:#x}"),
        result,
        "catalogue stream stopped: its last holder let it go"
    );
    reply_with(result)
}

/// `client` went away: it holds nothing any more, and each stream it was the
/// last to hold is stopped, a 1230 each, waited for, through the engine that
/// runs; with none running, nothing runs them, and none is started for
/// this. Its reader calls this once the client's requests have run, a stop
/// it sent before it went among them, and before it unlists the client, so
/// that the engine the client keeps up still runs (see
/// `daemon::client_reader`). The start a restore of its left to run
/// ([`start_restored`]) may not have run yet: the lock has it run before
/// this, its 1220 ahead of this 1230, or after, when it starts the stream
/// only for a client that holds it still. The 1230 is sent even while
/// other clients keep the engine up: the engine would otherwise run the
/// stream on, and start it again at each re-open. So the lock is waited for
/// as long as it takes, warned of once it passes the time a request waits
/// for it: whoever holds it gives it back within its commands' deadlines.
pub(crate) fn release(state: &Mutex<State>, client: u64) {
    // No request of the client's runs any more (its reader waits for its
    // request worker before calling this), so it cannot come to hold a
    // stream after this check: a restore's start makes nobody a holder.
    if !lock_state(state).catalogue.holds_any(client) {
        return;
    }
    let lock = catalogue_lock(state);
    let _held = lock_until(&lock, Instant::now() + LOCK_TIMEOUT).unwrap_or_else(|| {
        warn!(
            client,
            "a client that held a catalogue stream went away while another start or stop runs; waiting for it"
        );
        lock.lock().unwrap_or_else(PoisonError::into_inner)
    });
    let (emptied, device) = {
        let mut st = lock_state(state);
        let emptied = st.catalogue.release(client);
        (emptied, st.commands())
    };
    for stream in emptied {
        let Some(device) = device.as_deref() else {
            debug!(
                client,
                stream = %format_args!("{stream:#x}"),
                "the last holder of a catalogue stream went away; no engine runs it"
            );
            continue;
        };
        info!(
            client,
            stream = %format_args!("{stream:#x}"),
            "the last holder of a catalogue stream went away; stopping it"
        );
        let change = StreamChange::Stop(stream);
        if stream_result(change, device.change_stream(change, STREAM_TIMEOUT).wait())
            == Ok(tracker_error::FIRMWARE_NO_RESPONSE)
        {
            warn!(
                client,
                stream = %format_args!("{stream:#x}"),
                "the tracker did not answer a catalogue stream's stop in time"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Weak, mpsc};
    use std::thread;
    use tobii_ipc::deadline::worst;
    use tobii_ipc::request::{Request, StreamType, encode_stream_start, kind};
    use tobii_proto::facts::parse_stream_catalogue;
    use tobii_proto::protocol::{
        cmd, hex_to_bytes, parse_message, stream_id_payload, stream_start_payload, ttp_error,
    };
    use tobii_usb::engine::{CommandError, CommandResponse};

    use crate::daemon::tests::state_with_client;
    use crate::device::DeviceCommands;

    const CUSTOM: u32 = STREAM_ID_ALGODBG;
    const IMAGE_COLLECTION: u32 = STREAM_ID_IMAGE_COLLECTION;

    /// What befalls the daemon as the tracker answers a start.
    #[derive(Debug, Clone, Copy)]
    enum Mishap {
        /// The engine is lost, and another runs in its place (the stand-in
        /// stands for it).
        EngineLost,
        /// Client 2 restores its claim on the stream, as libtobii does when
        /// it reconnects.
        Restored(u32),
    }

    /// A command the tracker is to hold, unanswered, as it gets it, until
    /// the test lets it go (see [`meanwhile`]).
    struct Hold {
        /// A 1220 or a 1230.
        cmd: u32,
        /// Rung once the command is held.
        held: mpsc::Sender<()>,
        /// Lets it go once its sender is dropped.
        let_go: mpsc::Receiver<()>,
    }

    /// A stand-in for the tracker behind the engine: it logs every command
    /// with its deadline, answers a start (1220) with `start` and a stop
    /// (1230) with `stop`, and has `mishap` befall the daemon as it answers
    /// the next start. Given a `hold`, it holds the next command of its
    /// kind before it answers it.
    struct Tracker {
        sent: Mutex<Vec<(u32, Vec<u8>)>>,
        deadlines: Mutex<Vec<Duration>>,
        start: Mutex<Result<CommandResponse, CommandError>>,
        stop: Mutex<Result<CommandResponse, CommandError>>,
        mishap: Mutex<Option<Mishap>>,
        hold: Mutex<Option<Hold>>,
        state: Weak<Mutex<State>>,
    }

    impl DeviceCommands for Tracker {
        fn run(
            &self,
            cmd: u32,
            payload: Vec<u8>,
            timeout: Duration,
        ) -> Result<CommandResponse, CommandError> {
            self.sent.lock().expect("sent").push((cmd, payload));
            self.deadlines.lock().expect("deadlines").push(timeout);
            // Taken out, not locked across the wait: a command the daemon
            // sends meanwhile is answered at once.
            let hold = self.hold.lock().expect("hold").take_if(|h| h.cmd == cmd);
            if let Some(hold) = hold {
                let _ = hold.held.send(());
                // Bounded, should a test forget to let it go.
                let _ = hold.let_go.recv_timeout(Duration::from_secs(10));
            }
            if cmd != cmd::STREAM_START {
                return self.stop.lock().expect("stop").clone();
            }
            let mishap = self.mishap.lock().expect("mishap").take();
            if let (Some(mishap), Some(state)) = (mishap, self.state.upgrade()) {
                match mishap {
                    Mishap::EngineLost => lock_state(&state).drop_engine(),
                    Mishap::Restored(stream) => {
                        let _ = restore(&state, 2, stream);
                    }
                }
            }
            self.start.lock().expect("start").clone()
        }
    }

    struct Setup {
        state: Arc<Mutex<State>>,
        tracker: Arc<Tracker>,
    }

    /// The ET5's stream catalogue, as the init fixture's 1200 answer lists
    /// it: 0x1770 and 0x508 among its streams.
    fn et5_catalogue() -> Vec<StreamType> {
        let path = format!(
            "{}/../tobii-proto/fixtures/init-rsp-1200.hex",
            env!("CARGO_MANIFEST_DIR")
        );
        let bytes = hex_to_bytes(&std::fs::read_to_string(path).expect("fixture")).expect("hex");
        parse_stream_catalogue(&parse_message(&bytes).expect("msg"))
    }

    /// The facts of an init whose catalogue lists `streams`.
    fn listing(streams: &[u32]) -> Arc<DeviceFacts> {
        Arc::new(DeviceFacts {
            streams: streams
                .iter()
                .map(|&id| StreamType {
                    id,
                    ..StreamType::default()
                })
                .collect(),
            ..DeviceFacts::default()
        })
    }

    /// Clients 1 and 2, the facts of an init that got the ET5's catalogue,
    /// and a stand-in tracker that takes every command, so no engine
    /// starts.
    fn setup() -> Setup {
        let mut tracker = None;
        let state = Arc::new_cyclic(|weak| {
            let fake = Arc::new(Tracker {
                sent: Mutex::new(Vec::new()),
                deadlines: Mutex::new(Vec::new()),
                start: Mutex::new(Ok(CommandResponse::ok(Vec::new()))),
                stop: Mutex::new(Ok(CommandResponse::ok(Vec::new()))),
                mishap: Mutex::new(None),
                hold: Mutex::new(None),
                state: weak.clone(),
            });
            let mut st = state_with_client(1);
            st.clients.append(&mut state_with_client(2).clients);
            st.facts = Some(Arc::new(DeviceFacts {
                streams: et5_catalogue(),
                ..DeviceFacts::default()
            }));
            st.fake_device = Some(fake.clone());
            tracker = Some(fake);
            Mutex::new(st)
        });
        Setup {
            state,
            tracker: tracker.expect("tracker"),
        }
    }

    /// A request from `client`, through the daemon's dispatcher.
    fn ask(s: &Setup, client: u64, k: u8, payload: &[u8]) -> Reply {
        crate::requests::handle(
            &s.state,
            client,
            &Request {
                id: 1,
                kind: k,
                payload,
            },
        )
    }

    fn start_as(s: &Setup, client: u64, stream: u32) -> Reply {
        ask(
            s,
            client,
            kind::CATALOGUE_STREAM_START,
            &encode_stream_start(stream, false),
        )
    }

    fn restore_as(s: &Setup, client: u64, stream: u32) -> Reply {
        ask(
            s,
            client,
            kind::CATALOGUE_STREAM_START,
            &encode_stream_start(stream, true),
        )
    }

    fn stop_as(s: &Setup, client: u64, stream: u32) -> Reply {
        ask(s, client, kind::CATALOGUE_STREAM_STOP, &encode_u32(stream))
    }

    /// A restore's reply, which leaves `stream`'s start to run after it.
    fn restored(client: u64, stream: u32) -> Reply {
        Reply {
            then: Some(FollowUp::StartRestored { client, stream }),
            ..reply_with(tracker_error::NO_ERROR)
        }
    }

    /// Have the tracker answer the next starts `answer`.
    fn answer_start(s: &Setup, answer: Result<CommandResponse, CommandError>) {
        *s.tracker.start.lock().expect("start") = answer;
    }

    /// Have the tracker answer the next stops `answer`.
    fn answer_stop(s: &Setup, answer: Result<CommandResponse, CommandError>) {
        *s.tracker.stop.lock().expect("stop") = answer;
    }

    /// An answer with TTP `error`, and status 1 as every answer captured.
    fn refusal(error: u32) -> Result<CommandResponse, CommandError> {
        Ok(CommandResponse {
            error,
            ..CommandResponse::ok(Vec::new())
        })
    }

    /// Have `mishap` befall the daemon as the tracker answers the next
    /// start.
    fn befall(s: &Setup, mishap: Mishap) {
        *s.tracker.mishap.lock().expect("mishap") = Some(mishap);
    }

    /// How long a test waits for what must happen.
    const WAIT: Duration = Duration::from_secs(5);

    /// Run `first` on a thread of its own until the tracker holds the
    /// `cmd` it sends, then `second`, to its end, while that command is
    /// held, then let the command go: what each came to.
    fn meanwhile<T: Send>(
        s: &Setup,
        cmd: u32,
        first: impl FnOnce() -> T + Send,
        second: impl FnOnce() -> Reply,
    ) -> (T, Reply) {
        let (held_tx, held) = mpsc::channel();
        let (let_go, let_go_rx) = mpsc::channel::<()>();
        *s.tracker.hold.lock().expect("hold") = Some(Hold {
            cmd,
            held: held_tx,
            let_go: let_go_rx,
        });
        thread::scope(|scope| {
            let first = scope.spawn(first);
            held.recv_timeout(WAIT).expect("the command is held");
            let second = second();
            drop(let_go);
            (first.join().expect("answered"), second)
        })
    }

    fn sent(s: &Setup) -> Vec<(u32, Vec<u8>)> {
        s.tracker.sent.lock().expect("sent").clone()
    }

    /// The deadline of every command sent so far.
    fn deadlines(s: &Setup) -> Vec<Duration> {
        s.tracker.deadlines.lock().expect("deadlines").clone()
    }

    /// Forget what was sent so far.
    fn forget_sent(s: &Setup) {
        s.tracker.sent.lock().expect("sent").clear();
        s.tracker.deadlines.lock().expect("deadlines").clear();
    }

    fn holders(s: &Setup, stream: u32) -> Vec<u64> {
        lock_state(&s.state).catalogue.clients(stream).to_vec()
    }

    /// The 1220 of `stream`.
    fn start_of(stream: u32) -> (u32, Vec<u8>) {
        (cmd::STREAM_START, stream_start_payload(stream))
    }

    /// The 1230 of `stream`.
    fn stop_of(stream: u32) -> (u32, Vec<u8>) {
        (cmd::STREAM_STOP, stream_id_payload(stream))
    }

    /// A client holds a stream once, however often it starts it; the held
    /// streams come in id order, and the last holder to let one go empties
    /// it, by a stop or by going away.
    #[test]
    fn a_client_holds_a_stream_once_and_the_last_to_let_go_empties_it() {
        let mut h = Holders::default();
        h.add(CUSTOM, 7);
        h.add(CUSTOM, 7);
        h.add(IMAGE_COLLECTION, 9);
        h.add(CUSTOM, 9);

        assert_eq!(h.clients(CUSTOM), [7, 9], "each once");
        assert_eq!(h.held(), [IMAGE_COLLECTION, CUSTOM], "in id order");
        assert!(h.holds(CUSTOM, 7) && !h.holds(IMAGE_COLLECTION, 7));
        assert!(h.holds_any(9) && !h.holds_any(8));
        assert_eq!(h.remove(IMAGE_COLLECTION, 7), Removed::NotHeld);
        assert_eq!(h.remove(CUSTOM, 7), Removed::Others);
        assert_eq!(h.remove(CUSTOM, 7), Removed::NotHeld);
        assert_eq!(h.remove(0x500, 7), Removed::NotHeld);
        assert!(h.is_held(CUSTOM));

        assert_eq!(h.release(9), [IMAGE_COLLECTION, CUSTOM]);
        assert!(h.held().is_empty() && !h.is_held(CUSTOM) && !h.holds_any(9));
        assert_eq!(h.release(9), [], "nothing left");
        h.add(CUSTOM, 1);
        h.add(CUSTOM, 2);
        assert_eq!(h.release(1), [], "another holds it");
        assert_eq!(h.remove(CUSTOM, 2), Removed::Last);
        assert!(h.held().is_empty());
    }

    /// The custom stream's frames take the daemon's own routing bit, which
    /// no stream bit of the protocol takes, and image collection's none: a
    /// client gets the custom stream's frames while it holds the stream.
    #[test]
    fn a_holder_of_the_custom_stream_is_routed_its_frames() {
        use tobii_ipc::{
            RETIRED_STREAM_HEAD, STREAM_EYE_POSITION, STREAM_GAZE, STREAM_GAZE_DATA,
            STREAM_GAZE_ORIGIN, STREAM_GAZE_RAW, STREAM_HEAD_POSE, STREAM_IMAGE,
            STREAM_NOTIFICATIONS, STREAM_PRESENCE,
        };
        let protocol = [
            RETIRED_STREAM_HEAD,
            STREAM_GAZE,
            STREAM_PRESENCE,
            STREAM_GAZE_ORIGIN,
            STREAM_EYE_POSITION,
            STREAM_GAZE_DATA,
            STREAM_IMAGE,
            STREAM_NOTIFICATIONS,
            STREAM_GAZE_RAW,
            STREAM_HEAD_POSE,
        ];
        let mut h = Holders::default();
        h.add(CUSTOM, 1);
        h.add(IMAGE_COLLECTION, 2);
        h.add(IMAGE_COLLECTION, 1);

        assert_eq!(
            protocol.iter().fold(0, |mask, bit| mask | bit) & ROUTE_CUSTOM,
            0
        );
        assert_eq!(
            [CUSTOM, IMAGE_COLLECTION, 0x500].map(route),
            [ROUTE_CUSTOM, 0, 0]
        );
        assert_eq!([1, 2, 3].map(|c| h.routes_of(c)), [ROUTE_CUSTOM, 0, 0]);
    }

    /// The first client's start sends the one 1220 and answers what the
    /// tracker made of it; a second client's joins the stream with nothing
    /// sent. The first's stop sends nothing while the second holds the
    /// stream, and the second's sends the one 1230.
    #[test]
    fn two_clients_share_one_start_and_one_stop() {
        let s = setup();

        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(start_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM)], "joined");
        assert_eq!(holders(&s, CUSTOM), [1, 2]);

        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM)], "the other holds it");
        assert_eq!(stop_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)]);
        assert!(holders(&s, CUSTOM).is_empty());
        assert!(
            lock_state(&s.state).clients.iter().all(|c| c.holds_device),
            "each kept the device up, as any request does"
        );
    }

    /// A client that starts a stream it holds holds it still, with nothing
    /// sent, and its one stop lets it go.
    #[test]
    fn a_holder_starting_its_stream_again_sends_nothing() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(holders(&s, CUSTOM), [1]);
        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)]);
    }

    /// A stop from a client that does not hold the stream is answered at
    /// once, with nothing sent: no tracker is looked for, no facts waited
    /// for (there are none) and no engine started (none runs), and the
    /// client is not kept the device for.
    #[test]
    fn a_stop_from_a_client_that_holds_nothing_sends_nothing() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        forget_sent(&s);
        {
            let mut st = lock_state(&s.state);
            st.facts = None;
            st.fake_device = None;
        }

        assert_eq!(stop_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(
            stop_as(&s, 2, IMAGE_COLLECTION),
            reply_with(tracker_error::NO_ERROR)
        );

        assert!(sent(&s).is_empty());
        let st = lock_state(&s.state);
        assert_eq!((st.presence_probes, st.engines_started.len()), (0, 0));
        assert!(!st.clients[1].holds_device);
        assert_eq!(st.catalogue.clients(CUSTOM), [1]);
    }

    /// The last holder's stop with no engine running sends nothing, starts
    /// none, and lets the stream go: the next engine runs it no more.
    #[test]
    fn the_last_stop_without_an_engine_lets_the_stream_go() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        forget_sent(&s);
        lock_state(&s.state).fake_device = None;

        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

        assert!(sent(&s).is_empty());
        let mut st = lock_state(&s.state);
        assert!(st.engines_started.is_empty());
        st.ensure_engine(true);
        assert_eq!(st.engine_streams, [Vec::<u32>::new()]);
    }

    /// The last holder's stop lets the stream go whatever its 1230 comes to,
    /// and the reply carries what it came to: a refusal as the DLL gives
    /// it, no answer in time `FIRMWARE_NO_RESPONSE`, the tracker lost
    /// `CONNECTION_FAILED`. A stop the engine does not make (its catalogue
    /// does not list the stream, so it runs none of it) is `NO_ERROR`.
    #[test]
    fn the_last_stop_lets_the_stream_go_whatever_its_1230_comes_to() {
        for (answer, result) in [
            (refusal(ttp_error::UNAUTHORIZED), tracker_error::INTERNAL),
            (
                Err(CommandError::Timeout),
                tracker_error::FIRMWARE_NO_RESPONSE,
            ),
            (
                Err(CommandError::EngineGone),
                tracker_error::CONNECTION_FAILED,
            ),
            (Err(CommandError::NotSupported), tracker_error::NO_ERROR),
        ] {
            let s = setup();
            assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
            answer_stop(&s, answer.clone());

            assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(result), "{answer:?}");

            assert!(holders(&s, CUSTOM).is_empty(), "{answer:?}");
            assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)], "{answer:?}");
        }
    }

    /// A start that waited for another start or stop to finish joins a
    /// stream that came to be held meanwhile (a restore claimed it), with
    /// nothing sent: the restore's own start is to come.
    #[test]
    fn a_start_that_waited_for_the_lock_joins_a_stream_held_meanwhile() {
        let s = setup();
        let lock = catalogue_lock(&s.state);
        let busy = lock.lock().expect("lock");

        let reply = thread::scope(|scope| {
            let starting = scope.spawn(|| start_as(&s, 1, CUSTOM));
            thread::sleep(LOCK_TIMEOUT / 3);
            let _ = restore(&s.state, 2, CUSTOM);
            drop(busy);
            starting.join().expect("answered")
        });

        assert_eq!(reply, reply_with(tracker_error::NO_ERROR));
        assert!(sent(&s).is_empty());
        assert_eq!(holders(&s, CUSTOM), [2, 1]);
    }

    /// A stream the tracker's catalogue does not list is not supported, by
    /// a start or by a restore, and nothing is sent for it.
    #[test]
    fn a_stream_the_catalogue_does_not_list_is_not_supported() {
        let s = setup();
        lock_state(&s.state).facts = Some(listing(&[0x500, 0x504, CUSTOM]));

        assert_eq!(
            start_as(&s, 1, IMAGE_COLLECTION),
            Reply::err(status::NOT_SUPPORTED)
        );
        assert_eq!(
            restore_as(&s, 1, IMAGE_COLLECTION),
            Reply::err(status::NOT_SUPPORTED)
        );

        assert!(sent(&s).is_empty());
        assert!(holders(&s, IMAGE_COLLECTION).is_empty());
        // An init that lost its 1200, with none before it: no engine would
        // start the stream either.
        lock_state(&s.state).facts = Some(listing(&[]));
        assert_eq!(start_as(&s, 1, CUSTOM), Reply::err(status::NOT_SUPPORTED));
        assert_eq!(restore_as(&s, 1, CUSTOM), Reply::err(status::NOT_SUPPORTED));
        assert!(sent(&s).is_empty());
        assert!(holders(&s, CUSTOM).is_empty());
    }

    /// Only the custom stream and image collection are started or stopped:
    /// a start, restore or stop of any other stream is not supported, with
    /// nothing sent, though the catalogue lists it, the gaze and presence
    /// streams the init starts (a 1230 would stop gaze) and the image stream
    /// the engine runs among them.
    #[test]
    fn only_the_custom_stream_and_image_collection_are_started_or_stopped() {
        let s = setup();
        for stream in [0x500, 0x501, 0x504, 0x50e, 0x1771, 0x1774, 0] {
            assert_eq!(
                start_as(&s, 1, stream),
                Reply::err(status::NOT_SUPPORTED),
                "{stream:#x}"
            );
            assert_eq!(
                restore_as(&s, 1, stream),
                Reply::err(status::NOT_SUPPORTED),
                "{stream:#x}"
            );
            assert_eq!(
                stop_as(&s, 1, stream),
                Reply::err(status::NOT_SUPPORTED),
                "{stream:#x}"
            );
        }

        assert!(
            sent(&s).is_empty(),
            "no 1220 or 1230, for gaze least of all"
        );
        assert!(lock_state(&s.state).catalogue.held().is_empty());
    }

    #[test]
    fn a_start_or_stop_payload_is_a_stream_id() {
        let s = setup();
        for k in [kind::CATALOGUE_STREAM_START, kind::CATALOGUE_STREAM_STOP] {
            for payload in [&[][..], &[0x70, 0x17, 0]] {
                assert_eq!(
                    ask(&s, 1, k, payload),
                    Reply::err(status::INVALID_PARAMETER),
                    "{k}: {payload:?}"
                );
            }
        }
        assert!(sent(&s).is_empty());
        // A tail after the id (and a start's flags) is ignored.
        assert_eq!(
            ask(
                &s,
                1,
                kind::CATALOGUE_STREAM_START,
                &[0x70, 0x17, 0, 0, 0, 0xa5]
            ),
            reply_with(tracker_error::NO_ERROR)
        );
        assert_eq!(
            ask(
                &s,
                1,
                kind::CATALOGUE_STREAM_STOP,
                &[0x70, 0x17, 0, 0, 0xa5]
            ),
            reply_with(tracker_error::NO_ERROR)
        );
        assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)]);
    }

    /// A start the tracker refuses is answered its result, as the DLL gives
    /// it, and nobody holds the stream: the ET5 refuses image collection
    /// (0x20000505, INTERNAL). Nothing is sent to undo it, and the next
    /// client's start is sent again.
    #[test]
    fn a_start_the_tracker_refuses_leaves_the_stream_unheld() {
        let s = setup();
        answer_start(&s, refusal(ttp_error::UNAUTHORIZED));

        assert_eq!(
            start_as(&s, 1, IMAGE_COLLECTION),
            reply_with(tracker_error::INTERNAL)
        );

        assert_eq!(sent(&s), [start_of(IMAGE_COLLECTION)], "no 1230");
        assert!(holders(&s, IMAGE_COLLECTION).is_empty());
        assert_eq!(
            start_as(&s, 2, IMAGE_COLLECTION),
            reply_with(tracker_error::INTERNAL)
        );
        assert_eq!(
            sent(&s),
            [start_of(IMAGE_COLLECTION), start_of(IMAGE_COLLECTION)]
        );
        // Another refusal, its result as the DLL's table gives it.
        answer_start(&s, refusal(ttp_error::INVALID_PARAMETER));
        assert_eq!(
            start_as(&s, 1, CUSTOM),
            reply_with(tracker_error::INVALID_PARAMETER)
        );
        assert!(holders(&s, CUSTOM).is_empty());
    }

    /// An answer without an error is a start the tracker took, whatever its
    /// status word, which the DLL never reads.
    #[test]
    fn a_start_is_judged_by_its_error_word_alone() {
        for status_word in [0, 1, 2, u32::MAX] {
            let s = setup();
            answer_start(
                &s,
                Ok(CommandResponse {
                    status: status_word,
                    ..CommandResponse::ok(Vec::new())
                }),
            );

            assert_eq!(
                start_as(&s, 1, CUSTOM),
                reply_with(tracker_error::NO_ERROR),
                "status {status_word}"
            );
            assert_eq!(holders(&s, CUSTOM), [1], "status {status_word}");
        }
    }

    /// A start the tracker does not answer in time may have started the
    /// stream all the same: the reply says so, nobody holds the stream, and
    /// a 1230 follows the 1220, so that the engine keeps nothing running for
    /// nobody. Not when a client holds the stream by then: a restore claimed
    /// it meanwhile, and its own start is to come.
    #[test]
    fn a_start_not_answered_in_time_is_undone_unless_the_stream_is_held_by_then() {
        let s = setup();
        answer_start(&s, Err(CommandError::Timeout));

        assert_eq!(
            start_as(&s, 1, CUSTOM),
            reply_with(tracker_error::FIRMWARE_NO_RESPONSE)
        );

        assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)]);
        assert!(holders(&s, CUSTOM).is_empty());

        let s = setup();
        answer_start(&s, Err(CommandError::Timeout));
        befall(&s, Mishap::Restored(CUSTOM));

        assert_eq!(
            start_as(&s, 1, CUSTOM),
            reply_with(tracker_error::FIRMWARE_NO_RESPONSE)
        );

        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(holders(&s, CUSTOM), [2]);
    }

    /// A start counts for nothing when its engine is lost: one the lost
    /// engine took, as the one in its place was seeded without the stream,
    /// and one lost with it unanswered. Both are `CONNECTION_FAILED`, with
    /// nobody holding the stream and nothing sent to undo them: the lost
    /// engine stopped what it ran as it ended. A start whose open ended (or
    /// whose write failed) while the engine runs on may have started the
    /// stream there, and is undone.
    #[test]
    fn a_start_counts_for_nothing_when_its_engine_is_lost() {
        for (what, answer, lost, undone) in [
            (
                "taken, then lost",
                Ok(CommandResponse::ok(Vec::new())),
                true,
                false,
            ),
            (
                "lost unanswered",
                Err(CommandError::EngineGone),
                true,
                false,
            ),
            ("its open ended", Err(CommandError::EngineGone), false, true),
            (
                "not written",
                Err(CommandError::Usb("pipe".into())),
                false,
                true,
            ),
        ] {
            let s = setup();
            answer_start(&s, answer);
            if lost {
                befall(&s, Mishap::EngineLost);
            }

            assert_eq!(
                start_as(&s, 1, CUSTOM),
                reply_with(tracker_error::CONNECTION_FAILED),
                "{what}"
            );

            let mut expected = vec![start_of(CUSTOM)];
            if undone {
                expected.push(stop_of(CUSTOM));
            }
            assert_eq!(sent(&s), expected, "{what}");
            assert!(holders(&s, CUSTOM).is_empty(), "{what}");
        }
    }

    /// A start the engine does not make (its catalogue does not list the
    /// stream) is not supported, and nobody holds the stream.
    #[test]
    fn a_start_the_engine_does_not_make_is_not_supported() {
        let s = setup();
        answer_start(&s, Err(CommandError::NotSupported));

        assert_eq!(start_as(&s, 1, CUSTOM), Reply::err(status::NOT_SUPPORTED));

        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert!(holders(&s, CUSTOM).is_empty());
    }

    /// With no tracker on the bus a start fails at once, waiting neither
    /// for the facts (there are none: a wait for them would end `TIMED_OUT`
    /// 10 s later) nor for the lock, and starts no engine; the client keeps
    /// the device up all the same, so that the watchdog starts an engine
    /// once the tracker is plugged in. It holds nothing.
    #[test]
    fn a_start_without_a_tracker_fails_at_once() {
        let s = setup();
        {
            let mut st = lock_state(&s.state);
            st.fake_device = None;
            st.facts = None;
        }
        let lock = catalogue_lock(&s.state);
        let _busy = lock.lock().expect("lock");

        assert_eq!(
            start_as(&s, 1, CUSTOM),
            Reply::err(status::CONNECTION_FAILED)
        );

        let st = lock_state(&s.state);
        assert_eq!(
            st.presence_probes, 1,
            "the start's look at the bus, and no other"
        );
        assert!(st.engines_started.is_empty());
        assert!(st.clients[0].holds_device);
        assert!(st.catalogue.held().is_empty());
    }

    /// A start waits for the facts, then for another start or stop to
    /// finish, then for its 1220, and a stop for another start or stop and
    /// its 1230: within the worst cases libtobii's timeouts outlast.
    #[test]
    fn a_start_or_a_stop_waits_on_the_device_within_its_budget() {
        let s = setup();

        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

        let sent = deadlines(&s);
        assert_eq!(sent, [deadline::STREAM], "the DLL's 3 s for the answer");
        let waited = deadline::FACTS + deadline::STREAM_LOCK + deadline::commands(&sent);
        assert!(
            waited <= worst::CATALOGUE_STREAM_START,
            "{waited:?} in {:?}",
            worst::CATALOGUE_STREAM_START
        );
        forget_sent(&s);

        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

        let sent = deadlines(&s);
        assert_eq!(sent, [deadline::STREAM]);
        let waited = deadline::STREAM_LOCK + deadline::commands(&sent);
        assert!(
            waited <= worst::CATALOGUE_STREAM_STOP,
            "{waited:?} in {:?}",
            worst::CATALOGUE_STREAM_STOP
        );
    }

    /// A start or a stop waits only so long for another start or stop of a
    /// catalogue stream, then fails `TIMED_OUT` with nothing sent and
    /// nothing held or let go; a join, and a stop from a client that holds
    /// nothing, wait for none.
    #[test]
    fn a_start_or_a_stop_waits_for_another_only_so_long() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        forget_sent(&s);
        let lock = catalogue_lock(&s.state);
        let _busy = lock.lock().expect("lock");

        assert_eq!(
            start_as(&s, 2, IMAGE_COLLECTION),
            Reply::err(status::TIMED_OUT)
        );
        assert_eq!(stop_as(&s, 1, CUSTOM), Reply::err(status::TIMED_OUT));

        assert!(sent(&s).is_empty());
        assert_eq!(holders(&s, CUSTOM), [1], "still held");
        assert!(holders(&s, IMAGE_COLLECTION).is_empty());
        assert_eq!(start_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(holders(&s, CUSTOM), [1, 2]);
        assert_eq!(
            stop_as(&s, 2, IMAGE_COLLECTION),
            reply_with(tracker_error::NO_ERROR)
        );
        assert!(sent(&s).is_empty());
    }

    /// A start that comes while another client's start of the stream is on
    /// the device, the stream's first, waits for it: it sends no 1220 of
    /// its own while that one is unanswered (held there past the time a
    /// start waits for another, it gives up, `TIMED_OUT`), and one that
    /// comes once the tracker took the first joins the stream, with nothing
    /// sent.
    #[test]
    fn a_start_sends_no_1220_while_another_clients_start_is_on_the_device() {
        let s = setup();

        let (first, second) = meanwhile(
            &s,
            cmd::STREAM_START,
            || start_as(&s, 1, CUSTOM),
            || start_as(&s, 2, CUSTOM),
        );

        assert_eq!(second, Reply::err(status::TIMED_OUT));
        assert_eq!(first, reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(holders(&s, CUSTOM), [1]);
        assert_eq!(start_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(sent(&s), [start_of(CUSTOM)], "joined");
        assert_eq!(holders(&s, CUSTOM), [1, 2]);
    }

    /// The 1230 that undoes a start not answered in time is queued before
    /// the lock is let go, so that a start of the stream queued after it
    /// comes after it: another client's start that comes meanwhile, and
    /// finds the stream unheld, sends no 1220 until the 1230 is in (a
    /// stand-in takes a command in as it answers it; held past the time a
    /// start waits for another, the start gives up, `TIMED_OUT`), and the
    /// next start sends its 1220 after the 1230. The other way round, the
    /// 1230 would stop the stream that 1220 started.
    #[test]
    fn the_1230_undoing_a_start_is_queued_before_another_clients_start() {
        let s = setup();
        answer_start(&s, Err(CommandError::Timeout));

        let (undone, second) = meanwhile(
            &s,
            cmd::STREAM_STOP,
            || start_as(&s, 1, CUSTOM),
            || start_as(&s, 2, CUSTOM),
        );

        assert_eq!(second, Reply::err(status::TIMED_OUT));
        assert_eq!(undone, reply_with(tracker_error::FIRMWARE_NO_RESPONSE));
        assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)]);
        answer_start(&s, Ok(CommandResponse::ok(Vec::new())));
        assert_eq!(start_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(
            sent(&s),
            [start_of(CUSTOM), stop_of(CUSTOM), start_of(CUSTOM)]
        );
        assert_eq!(holders(&s, CUSTOM), [2]);
    }

    /// A start that comes while the last holder of the stream lets it go,
    /// by its stop or by going away, and finds the stream unheld, waits for
    /// that 1230 to be answered: it sends no 1220 while the 1230 is on the
    /// device (held there past the time a start waits for another, it gives
    /// up, `TIMED_OUT`), and the next start sends its 1220 after the 1230.
    /// The other way round, the engine would take the 1220 of a stream it
    /// runs with nothing written, and the 1230 would then stop the stream
    /// its new holder counts on.
    #[test]
    fn a_start_sends_no_1220_while_the_last_holders_1230_is_on_the_device() {
        for (what, going_away) in [("a stop", false), ("going away", true)] {
            let s = setup();
            assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

            let ((), second) = meanwhile(
                &s,
                cmd::STREAM_STOP,
                || {
                    if going_away {
                        release(&s.state, 1);
                    } else {
                        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
                    }
                },
                || start_as(&s, 2, CUSTOM),
            );

            assert_eq!(second, Reply::err(status::TIMED_OUT), "{what}");
            assert_eq!(sent(&s), [start_of(CUSTOM), stop_of(CUSTOM)], "{what}");
            assert!(holders(&s, CUSTOM).is_empty(), "{what}");
            assert_eq!(
                start_as(&s, 2, CUSTOM),
                reply_with(tracker_error::NO_ERROR),
                "{what}"
            );
            assert_eq!(
                sent(&s),
                [start_of(CUSTOM), stop_of(CUSTOM), start_of(CUSTOM)],
                "{what}"
            );
            assert_eq!(holders(&s, CUSTOM), [2], "{what}");
        }
    }

    /// A restore is answered at once from the daemon's own state: no look
    /// at the bus, no wait for the facts (there are none yet), no engine
    /// started and nothing sent. The client holds the stream, and keeps the
    /// device up, and the reply leaves the stream's start to run after it:
    /// the 1220 then.
    #[test]
    fn a_restore_is_answered_at_once_and_starts_its_stream_after() {
        let s = setup();
        lock_state(&s.state).facts = None;

        let reply = restore_as(&s, 1, CUSTOM);

        assert_eq!(reply, restored(1, CUSTOM));
        assert!(sent(&s).is_empty(), "nothing sent before the reply");
        {
            let st = lock_state(&s.state);
            assert_eq!((st.presence_probes, st.engines_started.len()), (0, 0));
            assert!(st.clients[0].holds_device);
            assert_eq!(st.catalogue.clients(CUSTOM), [1]);
        }
        reply.then.expect("a follow-up").run(&s.state);
        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(deadlines(&s), [deadline::STREAM]);
    }

    /// A restore of a stream another client holds joins it, with nothing
    /// to start after the reply; so does one of a stream the client holds.
    #[test]
    fn a_restore_of_a_held_stream_joins_it() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));

        assert_eq!(
            restore_as(&s, 2, CUSTOM),
            reply_with(tracker_error::NO_ERROR)
        );
        assert_eq!(
            restore_as(&s, 1, CUSTOM),
            reply_with(tracker_error::NO_ERROR)
        );

        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(holders(&s, CUSTOM), [1, 2]);
        assert!(lock_state(&s.state).clients[1].holds_device);
    }

    /// A restore is refused, as a start is, for a stream the known facts'
    /// catalogue does not list, facts without a catalogue among them: no
    /// engine would start the stream. With no facts yet it holds the stream.
    #[test]
    fn a_restore_is_refused_for_a_stream_the_known_facts_do_not_list() {
        for (what, facts, answer) in [
            (
                "listed",
                Some(listing(&[CUSTOM])),
                Some(restored(1, CUSTOM)),
            ),
            ("not listed", Some(listing(&[0x500])), None),
            ("no catalogue", Some(listing(&[])), None),
            ("no facts", None, Some(restored(1, CUSTOM))),
        ] {
            let s = setup();
            lock_state(&s.state).facts = facts;

            let reply = restore_as(&s, 1, CUSTOM);

            assert_eq!(
                reply,
                answer.unwrap_or_else(|| Reply::err(status::NOT_SUPPORTED)),
                "{what}"
            );
            assert_eq!(
                holders(&s, CUSTOM).is_empty(),
                reply.status != status::OK,
                "{what}"
            );
            assert_eq!(
                lock_state(&s.state).clients[0].holds_device,
                reply.status == status::OK,
                "{what}"
            );
        }
    }

    /// A restored stream the tracker does not start, refused or not
    /// answered, stays held: the client holds it whatever the tracker
    /// answers. A start not answered is not undone.
    #[test]
    fn a_restored_stream_stays_held_whatever_its_start_comes_to() {
        for answer in [
            refusal(ttp_error::UNAUTHORIZED),
            Err(CommandError::Timeout),
            Err(CommandError::EngineGone),
            Err(CommandError::NotSupported),
        ] {
            let s = setup();
            answer_start(&s, answer.clone());

            restore_as(&s, 1, CUSTOM)
                .then
                .expect("a follow-up")
                .run(&s.state);

            assert_eq!(sent(&s), [start_of(CUSTOM)], "{answer:?}");
            assert_eq!(holders(&s, CUSTOM), [1], "{answer:?}");
        }
    }

    /// With no engine running, a restore's start sends nothing: the next
    /// engine is started to run the stream.
    #[test]
    fn a_restored_stream_without_an_engine_is_left_to_the_next() {
        let s = setup();
        lock_state(&s.state).fake_device = None;

        restore_as(&s, 1, CUSTOM)
            .then
            .expect("a follow-up")
            .run(&s.state);

        assert!(sent(&s).is_empty());
        let mut st = lock_state(&s.state);
        assert!(st.engines_started.is_empty(), "none started for it");
        st.ensure_engine(true);
        assert_eq!(st.engine_streams, [vec![CUSTOM]]);
    }

    /// A restore's start waits for another start or stop for as long as it
    /// takes, past the time a request waits, and then starts the stream:
    /// given up on, the claim would stand with the stream not started.
    #[test]
    fn a_restores_start_waits_for_another_start_or_stop_to_finish() {
        let s = setup();
        let then = restore_as(&s, 1, CUSTOM).then.expect("a follow-up");
        let lock = catalogue_lock(&s.state);
        let busy = lock.lock().expect("lock");

        thread::scope(|scope| {
            let starting = scope.spawn(|| then.run(&s.state));
            thread::sleep(LOCK_TIMEOUT * 2);
            assert!(!starting.is_finished(), "still waiting");
            assert!(sent(&s).is_empty());
            drop(busy);
            starting.join().expect("started");
        });

        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(holders(&s, CUSTOM), [1]);
    }

    /// A restore's start starts the stream for a client that joined it
    /// meanwhile, though the client that restored it let it go: the join
    /// counted on that start, and nothing else would make it.
    #[test]
    fn a_restores_start_starts_the_stream_for_a_client_that_joined_meanwhile() {
        let s = setup();
        let then = restore_as(&s, 1, CUSTOM).then.expect("a follow-up");
        assert_eq!(start_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert!(sent(&s).is_empty(), "a join, then a stop the other holds");

        then.run(&s.state);

        assert_eq!(sent(&s), [start_of(CUSTOM)]);
        assert_eq!(holders(&s, CUSTOM), [2]);
    }

    /// A restore's start of a stream the client let go meanwhile, and
    /// nobody holds, sends nothing.
    #[test]
    fn a_restores_start_of_a_stream_let_go_sends_nothing() {
        let s = setup();
        let reply = restore_as(&s, 1, CUSTOM);
        assert_eq!(stop_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        forget_sent(&s);

        reply.then.expect("a follow-up").run(&s.state);

        assert!(sent(&s).is_empty());
    }

    /// A client going away lets go of what it holds: a stream another
    /// client holds runs on, with nothing sent, and each it was the last to
    /// hold is stopped, a 1230 each.
    #[test]
    fn the_last_holder_going_away_stops_the_stream() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(start_as(&s, 2, CUSTOM), reply_with(tracker_error::NO_ERROR));
        assert_eq!(
            start_as(&s, 2, IMAGE_COLLECTION),
            reply_with(tracker_error::NO_ERROR)
        );
        forget_sent(&s);

        release(&s.state, 1);
        assert!(sent(&s).is_empty(), "client 2 holds it");
        assert_eq!(holders(&s, CUSTOM), [2]);

        release(&s.state, 2);
        assert_eq!(sent(&s), [stop_of(IMAGE_COLLECTION), stop_of(CUSTOM)]);
        assert!(lock_state(&s.state).catalogue.held().is_empty());
        release(&s.state, 2);
        assert_eq!(sent(&s).len(), 2, "nothing left to stop");
    }

    /// A client going away with no engine running sends nothing and starts
    /// none; what it held is let go all the same.
    #[test]
    fn a_holder_going_away_without_an_engine_sends_nothing() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        forget_sent(&s);
        lock_state(&s.state).fake_device = None;

        release(&s.state, 1);

        assert!(sent(&s).is_empty());
        let st = lock_state(&s.state);
        assert!(st.catalogue.held().is_empty());
        assert_eq!((st.presence_probes, st.engines_started.len()), (0, 0));
    }

    /// A client going away waits for another start or stop for as long as
    /// it takes, past the time a request waits, and then stops what it was
    /// the last to hold: nobody else would.
    #[test]
    fn a_holder_going_away_waits_for_another_start_or_stop_to_finish() {
        let s = setup();
        assert_eq!(start_as(&s, 1, CUSTOM), reply_with(tracker_error::NO_ERROR));
        forget_sent(&s);
        let lock = catalogue_lock(&s.state);
        let busy = lock.lock().expect("lock");

        thread::scope(|scope| {
            let gone = scope.spawn(|| release(&s.state, 1));
            thread::sleep(LOCK_TIMEOUT * 2);
            assert!(!gone.is_finished(), "still waiting");
            assert!(sent(&s).is_empty());
            drop(busy);
            gone.join().expect("released");
        });

        assert_eq!(sent(&s), [stop_of(CUSTOM)]);
        assert!(holders(&s, CUSTOM).is_empty());
    }
}
