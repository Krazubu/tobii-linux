//! `state-probe`: the tracker commands behind Stream Engine 4.1's eleven
//! state-changing exports, planned and printed before anything is sent.
//!
//! Part A decoded every one of them from `tobii_stream_engine.dll` 4.1.0.3
//! (see `fixtures/analysis/state-commands-2026-10-21/REPORT-A.md`): the ids
//! were read from the request headers the `tracker_*` builders write, and
//! confirmed a second time by Ghidra's decompilation, which prints the
//! power-save id in decimal (`uStack_2fc = 3000`). Eight of the ten carry no
//! payload at all: the DLL's request writer reports `written ? written + 2 : 0`
//! (0x1800031e0), so a request with no TLV omits even the two `00 00` prefix
//! bytes and goes out as 24 bytes of body, 32 on the wire. Only command 1310
//! takes an argument here, the key name as TLV type 0x14 (0x18019641b), and a
//! string is written without a NUL.
//!
//! The whitelist is closed: [`PROBES`] holds exactly the commands the user
//! approved, and [`Plan::check`] refuses every other id before the tracker is
//! opened. Two commands the DLL can send are deliberately absent, and must
//! stay absent unless the user says otherwise:
//!
//! - **2020** `tracker_set_illumination_mode`. For a consumer ET5 the DLL
//!   never sends it: `platmod_ttp_set_illumination_mode` (0x180179150) needs
//!   module licence level 1 and `device_create` gives level 0, so the export
//!   answers `TOBII_ERROR_INSUFFICIENT_LICENSE` without any tracker traffic.
//!   libtobii answers the same, so there is nothing to learn from the wire.
//! - **1800** `tracker_enable_extension` (0x180194760). **It cannot be
//!   undone.** The image holds no disable string, no `platmod_ttp_disable_*`
//!   and no builder for one; the only extension verbs are enable 1800,
//!   enumerate 1810 and enumerate-enabled 1820.
//!
//! This module plans and prints; it sends nothing. Running the plan against
//! the tracker is a separate step, so the bytes can be checked against the
//! report first.

use anyhow::{Context as _, Result, bail};
use rusb::Context as UsbContext;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use tobii_proto::facts::parse_indexed_strings;
use tobii_proto::log::{PacketLog, log_packet};
use tobii_proto::protocol::{
    BulkReassembler, MARKER_NOTIFICATION, MARKER_RESPONSE, MARKER_STREAM, STREAM_ID_GAZE,
    chunk_command, cmd, parse_message, read_init_packets, stream_id,
};
use tobii_usb::device::{
    EP_IN, EP_OUT, Observed, READ_BUF, next_command_seq, open_tobii, replay_init_packets,
    send_command_observing, vendor_control_deinit, vendor_control_init, wait_for_gaze_stream,
};

/// Command 1490 `tracker_get_status`, the only command that reports the
/// power-save and remote-wake state: index 1 is power save and index 0 remote
/// wake (the DLL's index table at RVA 0x1a1610, its single call site
/// 0x18016dc94 inside `restart_after_reconnect`). Taken before and after
/// every step, since a successful write updates no cache the DLL can read.
pub(crate) const STATUS: u32 = cmd::STATUS;

/// Command 2010 `tracker_get_illumination_mode` (id at 0x18019aa25).
pub(crate) const ILLUMINATION_GET: u32 = 2010;

/// Command 2030 `tracker_enumerate_illumination_modes` (id at 0x180194424).
/// The DLL rejects an answer listing more than ten modes (0x18017bf7d,
/// refusal 0x180003d15).
pub(crate) const ILLUMINATION_ENUMERATE: u32 = 2030;

/// Command 1810 `tracker_enumerate_extensions` (id at 0x18019f674).
pub(crate) const EXTENSIONS_ENUMERATE: u32 = cmd::EXTENSIONS_ENUMERATE;

/// Command 1820 `tracker_enumerate_enabled_extensions` (id at 0x18019d114).
/// The only way to learn which extensions are on, since nothing disables one.
pub(crate) const EXTENSIONS_ENUMERATE_ENABLED: u32 = cmd::EXTENSIONS_ENUMERATE_ENABLED;

/// Command 1310 `tracker_get_configuration_key` (id at 0x180196456). Takes a
/// key name; the DLL supplies none of its own, so the name must come from the
/// caller.
pub(crate) const CONFIGURATION_KEY: u32 = 1310;

/// Command 3000 `tracker_power_save_activate` (id at 0x18019d803).
pub(crate) const POWER_SAVE_ACTIVATE: u32 = cmd::POWER_SAVE_ACTIVATE;

/// Command 3010 `tracker_power_save_deactivate` (id at 0x18019fc93).
pub(crate) const POWER_SAVE_DEACTIVATE: u32 = cmd::POWER_SAVE_DEACTIVATE;

/// Command 3040 `tracker_remote_wake_activate` (id at 0x18019d573).
pub(crate) const REMOTE_WAKE_ACTIVATE: u32 = cmd::REMOTE_WAKE_ACTIVATE;

/// Command 3050 `tracker_remote_wake_deactivate` (id at 0x1801941a3).
pub(crate) const REMOTE_WAKE_DEACTIVATE: u32 = cmd::REMOTE_WAKE_DEACTIVATE;

/// Command 2020, set illumination mode: refused, see the module docs.
const ILLUMINATION_SET: u32 = 2020;

/// Command 1800, enable extension: refused, see the module docs.
const EXTENSION_ENABLE: u32 = 1800;

/// The group of writes a user approves as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    /// Power save, commands 3000 and 3010.
    PowerSave,
    /// Remote wake, commands 3040 and 3050.
    RemoteWake,
}

impl Group {
    /// The group's name as `--group` takes it.
    #[must_use]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::PowerSave => "power-save",
            Self::RemoteWake => "remote-wake",
        }
    }

    /// The group `name` names, if any.
    #[must_use]
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "power-save" => Some(Self::PowerSave),
            "remote-wake" => Some(Self::RemoteWake),
            _ => None,
        }
    }

    /// The group's commands, in the order the probe plan sets out: the
    /// deactivate first, then the activate, then **the deactivate again**.
    ///
    /// The first one is the no-op that shows the answer's shape before
    /// anything changes, since the ET5's 1490 reported both states `false` in
    /// every answer seen. The last one is the revert, and it has to be a
    /// third step: the tracker is left activated otherwise, because the
    /// activate is what runs last.
    #[must_use]
    pub(crate) fn steps(self) -> [u32; 3] {
        let (deactivate, activate) = match self {
            Self::PowerSave => (POWER_SAVE_DEACTIVATE, POWER_SAVE_ACTIVATE),
            Self::RemoteWake => (REMOTE_WAKE_DEACTIVATE, REMOTE_WAKE_ACTIVATE),
        };
        [deactivate, activate, deactivate]
    }
}

/// What a command does to the tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A pure query: the DLL sends it without changing any tracker state.
    Read,
    /// Changes tracker state.
    Write {
        /// The group the user approves as a whole.
        group: Group,
        /// The command that must follow this one to put the state back, for
        /// a write that moves the tracker away from its resting state;
        /// `None` for the one that restores it.
        ///
        /// The halves are not symmetric. The ET5's 1490 reported both states
        /// `false` in every answer seen, so an activate leaves the tracker
        /// changed and needs its deactivate afterwards, while a deactivate
        /// restores rest and needs nothing after it. Treating the pair as
        /// symmetric is what let a group of `[deactivate, activate]` pass a
        /// revert check while leaving the tracker activated.
        restored_by: Option<u32>,
    },
}

/// What a command carries besides its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arg {
    /// No payload at all: 24 bytes of body, payload-length word zero.
    None,
    /// A key name, TLV type 0x14.
    KeyName,
}

/// One command the probe may send.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Probe {
    /// The DLL's name for the builder that writes it.
    pub(crate) name: &'static str,
    /// The TTP command id.
    pub(crate) id: u32,
    /// Read or write.
    pub(crate) kind: Kind,
    /// What it carries.
    pub(crate) arg: Arg,
    /// Where the DLL writes the id, for the report.
    pub(crate) dll: &'static str,
}

/// Every command the probe may send, and nothing else. Part A's whitelist,
/// as the user approved it on 2026-10-21: reads of both families whose
/// exports have no effective licence gate (2010, 2030, whose module functions
/// compare the licence level against 0), the extension reads the user chose
/// to answer past the DLL's licence refusal (1810, 1820), the configuration
/// key (1310), and the two write groups.
pub(crate) const PROBES: &[Probe] = &[
    Probe {
        name: "tracker_get_status",
        id: STATUS,
        kind: Kind::Read,
        arg: Arg::None,
        dll: "0x1801a0ac0, id at 0x1801a0b81",
    },
    Probe {
        name: "tracker_get_illumination_mode",
        id: ILLUMINATION_GET,
        kind: Kind::Read,
        arg: Arg::None,
        dll: "0x18019a820, id at 0x18019aa25",
    },
    Probe {
        name: "tracker_enumerate_illumination_modes",
        id: ILLUMINATION_ENUMERATE,
        kind: Kind::Read,
        arg: Arg::None,
        dll: "0x180194360, id at 0x180194424",
    },
    Probe {
        name: "tracker_enumerate_extensions",
        id: EXTENSIONS_ENUMERATE,
        kind: Kind::Read,
        arg: Arg::None,
        dll: "0x18019f5b0, id at 0x18019f674",
    },
    Probe {
        name: "tracker_enumerate_enabled_extensions",
        id: EXTENSIONS_ENUMERATE_ENABLED,
        kind: Kind::Read,
        arg: Arg::None,
        dll: "0x18019d050, id at 0x18019d114",
    },
    Probe {
        name: "tracker_get_configuration_key",
        id: CONFIGURATION_KEY,
        kind: Kind::Read,
        arg: Arg::KeyName,
        dll: "0x180196110, id at 0x180196456",
    },
    Probe {
        name: "tracker_power_save_deactivate",
        id: POWER_SAVE_DEACTIVATE,
        kind: Kind::Write {
            group: Group::PowerSave,
            // Restores the resting state the ET5 was seen in; nothing has
            // to follow it.
            restored_by: None,
        },
        arg: Arg::None,
        dll: "0x18019fbc0, id at 0x18019fc93",
    },
    Probe {
        name: "tracker_power_save_activate",
        id: POWER_SAVE_ACTIVATE,
        kind: Kind::Write {
            group: Group::PowerSave,
            restored_by: Some(POWER_SAVE_DEACTIVATE),
        },
        arg: Arg::None,
        dll: "0x18019d730, id at 0x18019d803",
    },
    Probe {
        name: "tracker_remote_wake_deactivate",
        id: REMOTE_WAKE_DEACTIVATE,
        kind: Kind::Write {
            group: Group::RemoteWake,
            // Restores the resting state the ET5 was seen in; nothing has
            // to follow it.
            restored_by: None,
        },
        arg: Arg::None,
        dll: "0x1801940d0, id at 0x1801941a3",
    },
    Probe {
        name: "tracker_remote_wake_activate",
        id: REMOTE_WAKE_ACTIVATE,
        kind: Kind::Write {
            group: Group::RemoteWake,
            restored_by: Some(REMOTE_WAKE_DEACTIVATE),
        },
        arg: Arg::None,
        dll: "0x18019d4a0, id at 0x18019d573",
    },
];

/// The whitelisted command `id` names, if any.
#[must_use]
pub(crate) fn probe(id: u32) -> Option<&'static Probe> {
    PROBES.iter().find(|p| p.id == id)
}

/// Why a command the DLL can send is kept out of the whitelist, if it is one
/// of those two.
fn refusal_reason(id: u32) -> Option<&'static str> {
    match id {
        ILLUMINATION_SET => Some(
            "2020 set illumination mode: a consumer ET5 never reaches it, the module needs \
             licence level 1 (0x18017915f) and device_create gives 0, so the export answers \
             TOBII_ERROR_INSUFFICIENT_LICENSE with no tracker traffic and libtobii answers the same",
        ),
        EXTENSION_ENABLE => Some(
            "1800 enable extension: there is no way to undo it. The image holds no disable \
             string, no platmod_ttp_disable_* and no builder for one; only enable 1800, \
             enumerate 1810 and enumerate-enabled 1820 exist",
        ),
        _ => None,
    }
}

/// The payload of a command that carries a key name: `00 00`, then TLV type
/// 0x14 with the string's length plus four, then the length again, then the
/// bytes, **without** a NUL. The DLL's string writer (0x180003fd0, type byte
/// at 0x180003ff9) builds it so.
#[must_use]
pub(crate) fn key_name_payload(key: &str) -> Vec<u8> {
    let bytes = key.as_bytes();
    // A key name is a short identifier; the cast cannot truncate in practice,
    // and `Plan::check` bounds it.
    let n = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    let mut payload = Vec::with_capacity(2 + 1 + 4 + 4 + bytes.len());
    payload.extend_from_slice(&[0x00, 0x00]);
    payload.push(tobii_proto::tlv::TYPE_STRING);
    payload.extend_from_slice(&n.saturating_add(4).to_be_bytes());
    payload.extend_from_slice(&n.to_be_bytes());
    payload.extend_from_slice(bytes);
    payload
}

/// Longest key name the probe will put on the wire.
const KEY_NAME_MAX: usize = 64;

/// The bytes one step writes to endpoint 0x05, prefix included.
///
/// # Errors
///
/// A command outside [`PROBES`], a key name where none is wanted or missing
/// where one is, or a payload that would need more than one bulk write (which
/// none of these commands can reach).
pub(crate) fn packet(id: u32, seq: u32, key: Option<&str>) -> Result<Vec<u8>> {
    let Some(p) = probe(id) else {
        bail!("{id} is not in the state-probe whitelist");
    };
    let payload = match (p.arg, key) {
        (Arg::None, None) => Vec::new(),
        (Arg::None, Some(_)) => bail!("command {id} takes no key name"),
        (Arg::KeyName, None) => bail!("command {id} needs a key name"),
        (Arg::KeyName, Some(k)) => {
            // Bounded here as well as in `Plan::check`, so building bytes is
            // safe for any caller: the TLV length words are `u32`, and a key
            // longer than this would have to saturate them. An empty name is
            // allowed on purpose: the DLL null-checks only the pointer
            // (0x18014c8d7), so a zero-length string is a request it would
            // make, and asking the tracker what it answers to a name it
            // cannot have is one of the approved probes.
            if k.len() > KEY_NAME_MAX {
                bail!(
                    "a key name must be at most {KEY_NAME_MAX} bytes, got {}",
                    k.len()
                );
            }
            key_name_payload(k)
        }
    };
    let mut writes = chunk_command(id, seq, &payload);
    if writes.len() != 1 {
        bail!("command {id} would need {} bulk writes", writes.len());
    }
    Ok(writes.swap_remove(0))
}

/// The configuration-key names the user approved on 2026-10-21, after a search
/// established that the DLL holds none of its own: its only `config` strings
/// belong to the bundled OpenSSL, no sibling command can enumerate the keys
/// (every `tracker_*` builder with an id in 1200..=1420 is 1200, 1210, 1310,
/// 1330, 1350, 1400 or 1420), nothing inside the DLL calls the export, and a
/// sweep of all eight captures (25 572 reassembled messages) turned up no key
/// name either.
///
/// So these are not invented: `HELLO_RGB` is key 4 of the tracker's own 1490
/// answer and `HELLO_NIR` is property 2 of its 1330 answer, both of which read
/// like configuration identifiers. The empty name asks what the tracker
/// answers for a name it cannot have, which tells a missing key apart from a
/// refusal for want of a realm or a licence.
pub(crate) const APPROVED_CONFIG_KEYS: [&str; 3] = ["HELLO_RGB", "HELLO_NIR", ""];

/// One command the probe would send, with the argument it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    /// The TTP command id, which must be in [`PROBES`].
    pub(crate) id: u32,
    /// The key name, for a command that takes one.
    pub(crate) key: Option<String>,
}

impl Step {
    /// A step for a command that carries nothing.
    #[must_use]
    pub(crate) fn plain(id: u32) -> Self {
        Self { id, key: None }
    }
}

/// What the probe intends to send, in order.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The commands, in the order they would go out. A 1490 is taken before
    /// and after each of them, which [`Plan::render`] shows.
    pub(crate) steps: Vec<Step>,
    /// The user has approved sending the write commands.
    pub(crate) allow_writes: bool,
}

impl Plan {
    /// Every step of the approved groups and reads.
    ///
    /// The plan comes back with `allow_writes` **false**, whatever groups it
    /// holds: the caller must set it from the user's `--allow-writes`, as
    /// [`run`] does, or [`Plan::check`] will refuse every write. Naming a
    /// group is not approval to send it.
    #[must_use]
    pub(crate) fn for_groups(groups: &[Group], reads: bool, keys: &[String]) -> Self {
        let mut steps = Vec::new();
        if reads {
            steps.extend(
                PROBES
                    .iter()
                    .filter(|p| p.kind == Kind::Read && p.arg == Arg::None)
                    .map(|p| Step::plain(p.id)),
            );
        }
        // One 1310 per key name, in the order the user gave them.
        steps.extend(keys.iter().map(|k| Step {
            id: CONFIGURATION_KEY,
            key: Some(k.clone()),
        }));
        for g in groups {
            steps.extend(g.steps().into_iter().map(Step::plain));
        }
        Self {
            steps,
            // Fail closed: a plan that holds writes still refuses them until
            // a caller sets this from the user's `--allow-writes`. The
            // constructor must never be the thing that permits a write.
            allow_writes: false,
        }
    }

    /// Refuse what the probe must not send, before the tracker is opened.
    ///
    /// # Errors
    ///
    /// A command outside the whitelist (naming why, for the two the DLL can
    /// send but we will not), a write without `--allow-writes`, a write whose
    /// revert is not in the same run, command 1310 without a key name or with
    /// an over-long one, or an empty plan.
    pub(crate) fn check(&self) -> Result<()> {
        if self.steps.is_empty() {
            bail!("nothing to send: name a group with --group or ask for the reads with --reads");
        }
        for (at, step) in self.steps.iter().enumerate() {
            let id = step.id;
            if let Some(why) = refusal_reason(id) {
                bail!("refusing {why}");
            }
            let Some(p) = probe(id) else {
                bail!(
                    "{id} is not in the state-probe whitelist, which holds only the commands \
                     part A decoded and the user approved: {}",
                    PROBES
                        .iter()
                        .map(|p| p.id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            };
            if let Kind::Write { group, restored_by } = p.kind {
                if !self.allow_writes {
                    bail!(
                        "{} ({}) changes tracker state; group {} needs --allow-writes",
                        p.name,
                        id,
                        group.as_str()
                    );
                }
                // The revert has to come *after* this write. A group opens
                // with its deactivate too, as the no-op that shows the
                // answer's shape, and that one must not be mistaken for the
                // revert: the tracker would be left changed.
                if let Some(revert) = restored_by
                    && !self.steps[at + 1..].iter().any(|s| s.id == revert)
                {
                    bail!(
                        "{} ({}) would be left in effect: its revert {revert} does not follow \
                         it in this run",
                        p.name,
                        id
                    );
                }
            }
            match (p.arg, step.key.as_deref()) {
                (Arg::KeyName, None) => bail!(
                    "command {id} needs a key name (--config-key or --approved-keys); the DLL \
                     supplies none of its own, so there is no default to fall back on"
                ),
                (Arg::KeyName, Some(key)) if key.len() > KEY_NAME_MAX => bail!(
                    "a key name must be at most {KEY_NAME_MAX} bytes, got {}",
                    key.len()
                ),
                (Arg::None, Some(_)) => bail!("command {id} takes no key name"),
                _ => {}
            }
        }
        Ok(())
    }

    /// The plan as the dry run prints it: every command with its bytes, so
    /// they can be checked against the report before anything is sent.
    ///
    /// # Errors
    ///
    /// Whatever [`packet`] refuses.
    pub(crate) fn render(&self) -> Result<String> {
        let mut s = String::new();
        // Steps, not distinct ids: a group sends its deactivate twice, and
        // counting ids would report "3 commands, 2 of them writes" for a run
        // in which all three are writes.
        let writes = self
            .steps
            .iter()
            .filter(|s| probe(s.id).is_some_and(|p| !matches!(p.kind, Kind::Read)))
            .count();
        let groups: BTreeSet<&str> = self
            .steps
            .iter()
            .filter_map(|s| match probe(s.id).map(|p| p.kind) {
                Some(Kind::Write { group, .. }) => Some(group.as_str()),
                _ => None,
            })
            .collect();
        let _ = writeln!(
            s,
            "state-probe dry run: {} command(s), {writes} of them writes{}. Nothing is sent.",
            self.steps.len(),
            if groups.is_empty() {
                String::new()
            } else {
                format!(
                    " (group(s) {})",
                    groups.iter().copied().collect::<Vec<_>>().join(", ")
                )
            }
        );
        let _ = writeln!(
            s,
            "A 1490 tracker_get_status is taken before and after each step; it is the only \
             command that reports the power-save (index 1) and remote-wake (index 0) state."
        );
        for (seq, step) in (1u32..).zip(self.steps.iter()) {
            let id = step.id;
            // `check` has already accepted every step.
            let p = probe(id).ok_or_else(|| anyhow::anyhow!("{id} left the whitelist"))?;
            let key = step.key.as_deref();
            let bytes = packet(id, seq, key)?;
            let kind = match p.kind {
                Kind::Read => "read".to_owned(),
                Kind::Write { group, .. } => format!("WRITE, group {}", group.as_str()),
            };
            let _ = writeln!(s, "\n-- {} ({id}, {id:#x}) [{kind}]", p.name);
            let _ = writeln!(s, "   DLL: {}", p.dll);
            if let Some(k) = key {
                let _ = writeln!(s, "   key name: {k:?} as TLV type 0x14, no NUL");
            }
            let _ = writeln!(s, "   EP 0x05, {} bytes: {}", bytes.len(), hex(&bytes));
        }
        Ok(s)
    }
}

/// Bytes as the report writes them: lowercase hex, grouped in fours.
fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            s.push(' ');
        }
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Plan the probe and print it. Sends nothing: the live run is a separate
/// step, so the bytes can be checked against part A's report first.
///
/// # Errors
///
/// Whatever [`Plan::check`] refuses, or a command whose bytes cannot be built.
pub(crate) fn run(command: &crate::cli::Command) -> Result<()> {
    let crate::cli::Command::StateProbe {
        groups,
        reads,
        allow_writes,
        config_keys,
        live,
        log_path,
        init_path,
    } = command
    else {
        bail!("not a state-probe command");
    };
    let mut plan = Plan::for_groups(groups, *reads, config_keys);
    plan.allow_writes = *allow_writes;
    plan.check()?;
    print!("{}", plan.render()?);
    if !*live {
        return Ok(());
    }
    let log_path = log_path
        .as_deref()
        .context("--run writes every message it sees: give it --log <file.bin>")?;
    println!("\n=== --run: opening the tracker and sending the plan above ===");
    execute(&plan, log_path, init_path.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_payload_command_is_thirty_two_bytes_on_the_wire() {
        // Part A: the DLL's request writer omits even the `00 00` prefix when
        // no TLV was written, so the body is 24 bytes and the wire form 32.
        let bytes = packet(POWER_SAVE_ACTIVATE, 4, None).expect("packet");
        assert_eq!(bytes.len(), 32, "{}", hex(&bytes));
        // prefix, then the LE body length, then the six BE header words.
        assert_eq!(&bytes[..4], &[0, 0, 0, 0]);
        assert_eq!(&bytes[4..8], &0x18_u32.to_le_bytes());
        assert_eq!(&bytes[8..12], &0x51_u32.to_be_bytes());
        assert_eq!(&bytes[12..16], &4_u32.to_be_bytes());
        assert_eq!(&bytes[20..24], &3000_u32.to_be_bytes());
        // the payload-length word is zero
        assert_eq!(&bytes[28..32], &0_u32.to_be_bytes());
    }

    #[test]
    fn every_whitelisted_write_id_matches_the_dll() {
        // The ids Ghidra printed, power save in decimal.
        assert_eq!(POWER_SAVE_ACTIVATE, 3000);
        assert_eq!(POWER_SAVE_DEACTIVATE, 3010);
        assert_eq!(REMOTE_WAKE_ACTIVATE, 3040);
        assert_eq!(REMOTE_WAKE_DEACTIVATE, 3050);
        assert_eq!(POWER_SAVE_ACTIVATE, 0xbb8);
        assert_eq!(POWER_SAVE_DEACTIVATE, 0xbc2);
        assert_eq!(REMOTE_WAKE_ACTIVATE, 0xbe0);
        assert_eq!(REMOTE_WAKE_DEACTIVATE, 0xbea);
    }

    #[test]
    fn the_two_commands_with_no_probe_are_refused_by_name() {
        for (id, needle) in [(2020_u32, "INSUFFICIENT_LICENSE"), (1800, "no way to undo")] {
            let plan = Plan {
                steps: vec![Step::plain(id)],
                allow_writes: true,
            };
            let e = plan.check().expect_err("must refuse");
            assert!(e.to_string().contains(needle), "{id}: {e}");
        }
    }

    #[test]
    fn an_id_outside_the_whitelist_is_refused() {
        let plan = Plan {
            steps: vec![Step::plain(1220)],
            allow_writes: true,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(
            e.to_string().contains("not in the state-probe whitelist"),
            "{e}"
        );
    }

    #[test]
    fn a_write_without_the_flag_is_refused() {
        let plan = Plan {
            steps: vec![
                Step::plain(POWER_SAVE_DEACTIVATE),
                Step::plain(POWER_SAVE_ACTIVATE),
            ],
            allow_writes: false,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(e.to_string().contains("--allow-writes"), "{e}");
    }

    #[test]
    fn a_write_whose_revert_is_missing_is_refused() {
        let plan = Plan {
            steps: vec![Step::plain(POWER_SAVE_ACTIVATE)],
            allow_writes: true,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(e.to_string().contains("left in effect"), "{e}");
    }

    #[test]
    fn an_activate_owes_its_deactivate_until_that_has_gone_out() {
        let mut owed = Owed::default();
        owed.sending(POWER_SAVE_DEACTIVATE);
        owed.sending(ILLUMINATION_GET);
        assert_eq!(owed, Owed::default(), "a deactivate or a read owes nothing");
        owed.sending(POWER_SAVE_ACTIVATE);
        owed.sending(POWER_SAVE_ACTIVATE);
        owed.sending(REMOTE_WAKE_ACTIVATE);
        assert_eq!(
            owed.take(),
            [REMOTE_WAKE_DEACTIVATE, POWER_SAVE_DEACTIVATE],
            "the latest write's first, each once"
        );
        assert!(owed.take().is_empty(), "paid");
        owed.sending(POWER_SAVE_ACTIVATE);
        owed.sending(POWER_SAVE_DEACTIVATE);
        assert!(owed.take().is_empty(), "the plan paid it itself");
    }

    #[test]
    fn a_run_cut_short_after_an_activate_still_owes_its_deactivate() {
        // An interrupt or a failure can end the steps anywhere; what is owed
        // then is sent before the open ends. A whole plan owes nothing.
        let plan = Plan::for_groups(&[Group::PowerSave, Group::RemoteWake], true, &[]);
        let owed_after = |cut: usize| {
            let mut owed = Owed::default();
            for step in &plan.steps[..cut] {
                owed.sending(step.id);
            }
            owed.take()
        };
        assert!(owed_after(plan.steps.len()).is_empty());
        for (at, step) in plan.steps.iter().enumerate() {
            if let Some(Probe {
                kind: Kind::Write {
                    restored_by: Some(revert),
                    ..
                },
                ..
            }) = probe(step.id)
            {
                assert_eq!(owed_after(at + 1), [*revert], "cut after step {}", at + 1);
            }
        }
    }

    #[test]
    fn a_group_ends_with_its_revert_so_nothing_is_left_in_effect() {
        // Three steps, not two: the deactivate that opens the group is the
        // no-op that shows the answer's shape, and a second deactivate at
        // the end is what actually reverts the activate.
        assert_eq!(
            Group::PowerSave.steps(),
            [
                POWER_SAVE_DEACTIVATE,
                POWER_SAVE_ACTIVATE,
                POWER_SAVE_DEACTIVATE
            ]
        );
        assert_eq!(
            Group::RemoteWake.steps(),
            [
                REMOTE_WAKE_DEACTIVATE,
                REMOTE_WAKE_ACTIVATE,
                REMOTE_WAKE_DEACTIVATE
            ]
        );
        for group in [Group::PowerSave, Group::RemoteWake] {
            let steps = group.steps();
            let mut plan = Plan::for_groups(&[group], false, &[]);
            plan.allow_writes = true;
            plan.check().expect("a whole group passes");
            assert_eq!(
                plan.steps.last().map(|s| s.id),
                Some(steps[0]),
                "the run must end on the revert"
            );
        }
    }

    #[test]
    fn a_revert_that_only_precedes_the_write_is_refused() {
        // The bug this guards: a group of [deactivate, activate] satisfies a
        // naive "is the revert anywhere in the run" check while leaving the
        // tracker activated, because the activate is what runs last.
        let plan = Plan {
            steps: vec![
                Step::plain(POWER_SAVE_DEACTIVATE),
                Step::plain(POWER_SAVE_ACTIVATE),
            ],
            allow_writes: true,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(e.to_string().contains("does not follow it"), "{e}");
    }

    #[test]
    fn the_constructor_never_permits_a_write_by_itself() {
        // Fail closed: naming a group must not be what allows its writes.
        let plan = Plan::for_groups(&[Group::PowerSave, Group::RemoteWake], true, &[]);
        assert!(!plan.allow_writes);
        let e = plan.check().expect_err("must refuse until the flag is set");
        assert!(e.to_string().contains("--allow-writes"), "{e}");
    }

    #[test]
    fn the_command_allows_a_write_only_with_its_flag() {
        // What reaches the plan is the user's `--allow-writes`, nothing else.
        let probe = |allow_writes| crate::cli::Command::StateProbe {
            groups: vec![Group::PowerSave],
            reads: false,
            allow_writes,
            config_keys: Vec::new(),
            live: false,
            log_path: None,
            init_path: None,
        };
        let e = run(&probe(false)).expect_err("a write group without the flag");
        assert!(e.to_string().contains("--allow-writes"), "{e}");
        run(&probe(true)).expect("the same group with it");
    }

    #[test]
    fn both_groups_pass_together_and_keep_each_revert_first() {
        let mut plan = Plan::for_groups(&[Group::PowerSave, Group::RemoteWake], true, &[]);
        plan.allow_writes = true;
        plan.check().expect("both groups pass");
        let text = plan.render().expect("render");
        let at = |needle: &str| text.find(needle).expect("step is rendered");
        assert!(at("(3010,") < at("(3000,"), "{text}");
        assert!(at("(3050,") < at("(3040,"), "{text}");
    }

    #[test]
    fn one_step_per_key_name_and_the_reads_add_none_of_their_own() {
        let reads = Plan::for_groups(&[], true, &[]);
        assert!(
            !reads.steps.iter().any(|s| s.id == CONFIGURATION_KEY),
            "the reads must leave 1310 to the key names: {:?}",
            reads.steps
        );
        let keys: Vec<String> = APPROVED_CONFIG_KEYS
            .iter()
            .map(|k| (*k).to_owned())
            .collect();
        let plan = Plan::for_groups(&[], true, &keys);
        let got: Vec<Option<&str>> = plan
            .steps
            .iter()
            .filter(|s| s.id == CONFIGURATION_KEY)
            .map(|s| s.key.as_deref())
            .collect();
        assert_eq!(got, vec![Some("HELLO_RGB"), Some("HELLO_NIR"), Some("")]);
        plan.check().expect("the approved keys pass");
    }

    #[test]
    fn the_approved_keys_are_the_three_the_user_chose() {
        // Not invented: HELLO_RGB is key 4 of the tracker's 1490 answer,
        // HELLO_NIR property 2 of its 1330, and the empty name asks what it
        // answers for a name it cannot have.
        assert_eq!(APPROVED_CONFIG_KEYS, ["HELLO_RGB", "HELLO_NIR", ""]);
    }

    #[test]
    fn building_bytes_bounds_the_key_name_even_without_a_plan() {
        let long = "x".repeat(KEY_NAME_MAX + 1);
        let e = packet(CONFIGURATION_KEY, 1, Some(&long)).expect_err("must refuse");
        assert!(e.to_string().contains("at most"), "{e}");
        // An empty name is a request the DLL would make: it null-checks only
        // the pointer, so a zero-length string is legal and is one of the
        // approved probes.
        let bytes = packet(CONFIGURATION_KEY, 1, Some("")).expect("empty is allowed");
        assert_eq!(bytes.len(), 8 + 24 + 2 + 1 + 4 + 4);
        // Both ends of the accepted range, against an off-by-one.
        for key in ["a", &"x".repeat(KEY_NAME_MAX)] {
            let bytes = packet(CONFIGURATION_KEY, 1, Some(key)).expect("accepted");
            // prefix + body: header, the `00 00`, the type byte, two length
            // words and the key itself.
            assert_eq!(bytes.len(), 8 + 24 + 2 + 1 + 4 + 4 + key.len(), "{key:?}");
        }
    }

    #[test]
    fn the_configuration_key_needs_a_name_and_bounds_it() {
        let plan = Plan {
            steps: vec![Step::plain(CONFIGURATION_KEY)],
            allow_writes: false,
        };
        let e = plan.check().expect_err("no name");
        assert!(e.to_string().contains("--config-key"), "{e}");
        let plan = Plan {
            steps: vec![Step {
                id: CONFIGURATION_KEY,
                key: Some("x".repeat(KEY_NAME_MAX + 1)),
            }],
            allow_writes: false,
        };
        let e = plan.check().expect_err("too long");
        assert!(e.to_string().contains("at most"), "{e}");
        let plan = Plan::for_groups(&[], false, &["serial_number".to_owned()]);
        plan.check().expect("a short name passes");
    }

    #[test]
    fn a_command_that_takes_no_key_is_refused_a_key_by_the_plan() {
        let plan = Plan {
            steps: vec![Step {
                id: STATUS,
                key: Some("k".to_owned()),
            }],
            allow_writes: false,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(e.to_string().contains("takes no key name"), "{e}");
    }

    #[test]
    fn a_key_name_is_written_as_a_string_without_a_nul() {
        let payload = key_name_payload("ab");
        assert_eq!(
            payload,
            vec![0x00, 0x00, 0x14, 0, 0, 0, 6, 0, 0, 0, 2, b'a', b'b']
        );
    }

    #[test]
    fn a_command_that_takes_no_key_refuses_one() {
        let e = packet(POWER_SAVE_ACTIVATE, 1, Some("k")).expect_err("must refuse");
        assert!(e.to_string().contains("takes no key name"), "{e}");
    }

    #[test]
    fn an_empty_plan_is_refused() {
        let plan = Plan {
            steps: Vec::new(),
            allow_writes: false,
        };
        assert!(plan.check().is_err());
    }

    #[test]
    fn the_reads_plan_holds_no_write_and_passes() {
        let plan = Plan::for_groups(&[], true, &[]);
        assert!(!plan.steps.is_empty());
        for step in &plan.steps {
            let id = step.id;
            assert_eq!(probe(id).expect("whitelisted").kind, Kind::Read, "{id}");
        }
        plan.check().expect("reads alone pass");
    }

    #[test]
    fn the_rendered_plan_shows_the_bytes_and_marks_the_writes() {
        let plan = Plan::for_groups(&[Group::RemoteWake], true, &[]);
        let text = plan.render().expect("render");
        assert!(
            text.contains("tracker_remote_wake_activate (3040, 0xbe0)"),
            "{text}"
        );
        assert!(text.contains("WRITE, group remote-wake"), "{text}");
        assert!(text.contains("tracker_get_status"), "{text}");
        // the zero-payload wire form, as the report documents it
        assert!(text.contains("00000000 18000000 00000051"), "{text}");
    }
}

// ---------------------------------------------------------------------------
// The live run.
// ---------------------------------------------------------------------------

/// Opens tried before giving up: the first open after a cold start may not
/// arm the gaze stream, as `stream-probe` and `image83` both see.
const OPEN_ATTEMPTS: u32 = 3;

/// How long an open waits for its first gaze frame.
const ARM_TIMEOUT: Duration = Duration::from_millis(4500);

/// How long a command waits for its response once written. The tracker's own
/// deadline for these is 3 s in the DLL; this is the probe's patience for the
/// answer arriving on EP 0x83.
const RESPONSE_TIMEOUT: Duration = Duration::from_millis(3500);

/// One read of EP 0x83 inside a window.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// How long EP 0x83 is read after a step, to catch a notification the step
/// provoked (3020/3030 for power save) and to see whether gaze kept coming.
const AFTER_STEP: Duration = Duration::from_millis(1500);

/// Set by the SIGINT/SIGTERM handler: the run stops starting new steps, sends
/// what it still owes and ends the open rather than leave the tracker
/// changed. A pure signal, so every access is `Ordering::Relaxed`.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_interrupt(_sig: libc::c_int) {
    // Async-signal-safe: only an atomic store.
    INTERRUPTED.store(true, Ordering::Relaxed);
}

/// Catch SIGINT and SIGTERM so an interrupted run still reverts and tears down.
///
/// # Errors
///
/// The handler could not be installed.
fn catch_interrupts() -> Result<()> {
    let handler: extern "C" fn(libc::c_int) = on_interrupt;
    for signal in [libc::SIGINT, libc::SIGTERM] {
        // SAFETY: `handler` is an `extern "C" fn(c_int)`, what `sighandler_t`
        // stands for, and only performs an atomic store, so it is
        // async-signal-safe; the default handler it replaces needs no
        // restoring.
        let previous = unsafe { libc::signal(signal, handler as libc::sighandler_t) };
        if previous == libc::SIG_ERR {
            return Err(std::io::Error::last_os_error()).context("catching SIGINT and SIGTERM");
        }
    }
    Ok(())
}

/// What the tracker's 1490 answer says about the two states this probe moves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Status {
    /// Index 1 of the answer: power save active.
    pub(crate) power_save: Option<String>,
    /// Index 0 of the answer: remote wake active.
    pub(crate) remote_wake: Option<String>,
}

impl Status {
    /// The two states of a 1490 response, by the DLL's index table
    /// (RVA 0x1a1610): index 1 is power save, index 0 remote wake.
    #[must_use]
    pub(crate) fn of_response(msg: &[u8]) -> Self {
        let Some(m) = parse_message(msg) else {
            return Self::default();
        };
        let items = parse_indexed_strings(&m);
        let at = |index: u32| {
            items
                .iter()
                .find(|(i, _)| *i == index)
                .map(|(_, v)| v.clone())
        };
        Self {
            power_save: at(1),
            remote_wake: at(0),
        }
    }

    /// The pair as the report prints it.
    fn render(&self) -> String {
        let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "?".to_owned());
        format!(
            "power save {}, remote wake {}",
            show(&self.power_save),
            show(&self.remote_wake)
        )
    }
}

/// What one window of EP 0x83 held.
#[derive(Debug, Default)]
struct Seen {
    /// Gaze frames (stream 0x500).
    gaze: usize,
    /// Notification ids, in order.
    notifications: Vec<u32>,
    /// Responses that were not the one being waited for.
    stray: usize,
}

impl Seen {
    /// Tally one reassembled message.
    fn add(&mut self, msg: &[u8]) {
        match parse_message(msg) {
            Some(m) if m.marker == MARKER_STREAM => {
                if stream_id(msg) == Some(STREAM_ID_GAZE) {
                    self.gaze += 1;
                }
            }
            Some(m) if m.marker == MARKER_NOTIFICATION => self.notifications.push(m.id),
            Some(m) if m.marker == MARKER_RESPONSE => self.stray += 1,
            _ => {}
        }
    }
}

/// Send one command and wait for its response, reading EP 0x83 throughout:
/// an unread IN message stalls the tracker's OUT endpoint.
fn command(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    seen: &mut Seen,
    packet: &[u8],
) -> Result<Option<Vec<u8>>> {
    let mut logged = Ok(());
    let response = send_command_observing(h, packet, RESPONSE_TIMEOUT, asm, |event| {
        let record = match event {
            Observed::Written => log_packet(log, EP_OUT, packet),
            Observed::Read(m) => {
                seen.add(m);
                log_packet(log, EP_IN, m)
            }
        };
        if logged.is_ok() {
            logged = record;
        }
    });
    let response = response?;
    logged?;
    Ok(response)
}

/// Read EP 0x83 for `dur`, logging and tallying everything.
fn read_window(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    seen: &mut Seen,
    dur: Duration,
) -> Result<()> {
    let mut buf = vec![0u8; READ_BUF];
    let mut msgs = Vec::new();
    let start = Instant::now();
    while start.elapsed() < dur {
        let n = match h.read_bulk(EP_IN, &mut buf, READ_TIMEOUT) {
            Ok(n) => n,
            Err(rusb::Error::Timeout) => continue,
            Err(e) => return Err(e).context("reading EP 0x83 (did the tracker disconnect?)"),
        };
        asm.push_into(&buf[..n], &mut msgs);
        for m in &msgs {
            log_packet(log, EP_IN, m)?;
            seen.add(m);
        }
    }
    Ok(())
}

/// Take the tracker's status (command 1490) and report the two states.
fn status_of(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    seen: &mut Seen,
    seq: &mut u32,
) -> Result<Status> {
    let bytes = packet(STATUS, *seq, None)?;
    *seq += 1;
    let response = command(h, asm, log, seen, &bytes)?;
    Ok(response
        .map(|r| Status::of_response(&r))
        .unwrap_or_default())
}

/// The reverts a run owes the tracker: for each write it sent (or tried to:
/// a write whose answer never came may still have been carried out), the
/// command that puts its state back, until that command has gone out too.
#[derive(Debug, Default, PartialEq, Eq)]
struct Owed(Vec<u32>);

impl Owed {
    /// Note that `id` is about to go out: it pays its own debt, if it is a
    /// revert, and runs one up, if it is a write that needs one.
    fn sending(&mut self, id: u32) {
        self.0.retain(|&revert| revert != id);
        if let Some(Probe {
            kind: Kind::Write {
                restored_by: Some(revert),
                ..
            },
            ..
        }) = probe(id)
            && !self.0.contains(revert)
        {
            self.0.push(*revert);
        }
    }

    /// The reverts still owed, the latest write's first, and none after.
    fn take(&mut self) -> Vec<u32> {
        let mut owed = std::mem::take(&mut self.0);
        owed.reverse();
        owed
    }
}

/// Run the plan's steps, each sandwiched between two 1490s. Returns the
/// report built so far and the first error, so the caller can still tear the
/// open down and print what happened. However the steps end, interrupted or
/// failed, every revert still owed is sent before this returns.
fn run_steps(
    h: &mut rusb::DeviceHandle<UsbContext>,
    asm: &mut BulkReassembler,
    log: &mut Option<PacketLog>,
    plan: &Plan,
    seq: &mut u32,
) -> (String, Result<()>) {
    let mut out = String::new();
    let mut owed = Owed::default();
    let mut outcome = Ok(());
    let mut run = Run {
        h,
        asm,
        log,
        seq,
        out: &mut out,
    };
    for (at, step) in plan.steps.iter().enumerate() {
        if INTERRUPTED.load(Ordering::Relaxed) {
            let _ = writeln!(run.out, "\n-- interrupted before step {}", at + 1);
            break;
        }
        if let Err(e) = run.step(plan, at, step, &mut owed) {
            outcome = Err(e);
            break;
        }
    }
    run.pay(&mut owed);
    (out, outcome)
}

/// What a step needs of the open, and the report it adds to.
struct Run<'a> {
    h: &'a mut rusb::DeviceHandle<UsbContext>,
    asm: &'a mut BulkReassembler,
    log: &'a mut Option<PacketLog>,
    seq: &'a mut u32,
    out: &'a mut String,
}

impl Run<'_> {
    /// Step `at` of `plan`: the 1490 before, the command, a window to catch
    /// what it provoked, and the 1490 after.
    fn step(&mut self, plan: &Plan, at: usize, step: &Step, owed: &mut Owed) -> Result<()> {
        let Some(p) = probe(step.id) else {
            bail!("{} left the whitelist", step.id);
        };
        let mut seen = Seen::default();
        let before = status_of(self.h, self.asm, self.log, &mut seen, self.seq)
            .context("the 1490 before a step")?;
        let bytes = packet(step.id, *self.seq, step.key.as_deref())?;
        *self.seq += 1;
        let _ = writeln!(
            self.out,
            "\n-- step {} of {}: {} ({}, {:#x}){}",
            at + 1,
            plan.steps.len(),
            p.name,
            step.id,
            step.id,
            step.key
                .as_deref()
                .map_or(String::new(), |k| format!(" key {k:?}"))
        );
        let _ = writeln!(self.out, "   1490 before: {}", before.render());
        owed.sending(step.id);
        let answer = match command(self.h, self.asm, self.log, &mut seen, &bytes) {
            Ok(a) => a,
            Err(e) => {
                let _ = writeln!(self.out, "   FAILED to send: {e}");
                return Err(e);
            }
        };
        let _ = writeln!(self.out, "   answer: {}", render_answer(answer.as_deref()));
        read_window(self.h, self.asm, self.log, &mut seen, AFTER_STEP)?;
        let after = status_of(self.h, self.asm, self.log, &mut seen, self.seq)
            .context("the 1490 after a step")?;
        let _ = writeln!(self.out, "   1490 after:  {}", after.render());
        if before != after {
            let _ = writeln!(self.out, "   *** the status CHANGED ***");
        }
        let _ = writeln!(
            self.out,
            "   in {:.1} s after: {} gaze frame(s), notifications {:?}",
            AFTER_STEP.as_secs_f64(),
            seen.gaze,
            seen.notifications
        );
        if seen.gaze == 0 {
            let _ = writeln!(self.out, "   *** NO GAZE in the window after this step ***");
        }
        Ok(())
    }

    /// Send every revert still `owed`, whatever came of the one before:
    /// leaving the tracker changed is the one outcome worse than an error.
    fn pay(&mut self, owed: &mut Owed) {
        for revert in owed.take() {
            let mut seen = Seen::default();
            let sent = packet(revert, *self.seq, None).and_then(|bytes| {
                *self.seq += 1;
                command(self.h, self.asm, self.log, &mut seen, &bytes)
            });
            let _ = match sent {
                Ok(answer) => writeln!(
                    self.out,
                    "\n-- owed revert {revert}: {}",
                    render_answer(answer.as_deref())
                ),
                Err(e) => writeln!(self.out, "\n-- owed revert {revert}: FAILED to send: {e}"),
            };
        }
    }
}

/// One line on a command's answer, or on its absence.
fn render_answer(answer: Option<&[u8]>) -> String {
    match answer.and_then(parse_message) {
        Some(m) => format!(
            "status {} error {:#010x}, {} B payload",
            m.status,
            m.error,
            m.payload.len()
        ),
        None => format!("none within {RESPONSE_TIMEOUT:?}"),
    }
}

/// Open the tracker, replay tobiid's init, run the plan and tear the open
/// down, whatever happened.
///
/// # Errors
///
/// The device could not be opened, the init failed, the gaze stream never
/// armed, or a step failed.
fn execute(plan: &Plan, log_path: &str, init_path: Option<&str>) -> Result<()> {
    let packets = match init_path {
        Some(path) => read_init_packets(path)?,
        None => crate::stream_probe::daemon_init_packets()?,
    };
    let mut log = Some(PacketLog::create(log_path)?);
    catch_interrupts()?;
    let ctx = UsbContext::new()?;
    for attempt in 1..=OPEN_ATTEMPTS {
        if INTERRUPTED.load(Ordering::Relaxed) {
            bail!("interrupted before the run began");
        }
        let mut h = open_tobii(&ctx)?;
        vendor_control_init(&mut h)?;
        println!("open {attempt}: replaying {} init packets", packets.len());
        if let Err(e) = replay_init_packets(&mut h, &packets, Some(&INTERRUPTED)) {
            vendor_control_deinit(&mut h);
            return Err(e);
        }
        let mut asm = BulkReassembler::new();
        if !wait_for_gaze_stream(&mut h, &INTERRUPTED, ARM_TIMEOUT, &mut asm) {
            println!("no gaze frame within {ARM_TIMEOUT:?}; re-opening (cold-start quirk)");
            vendor_control_deinit(&mut h);
            drop(h);
            thread::sleep(Duration::from_millis(700));
            continue;
        }
        let mut seq = next_command_seq(&packets);
        let (report, outcome) = run_steps(&mut h, &mut asm, &mut log, plan, &mut seq);
        // The open always ends the way every Windows session ends.
        vendor_control_deinit(&mut h);
        print!("{report}");
        println!("\nlog: {log_path}");
        if INTERRUPTED.load(Ordering::Relaxed) {
            bail!("interrupted: the run stopped early, the open was torn down");
        }
        return outcome;
    }
    bail!("the gaze stream never armed in {OPEN_ATTEMPTS} opens")
}
