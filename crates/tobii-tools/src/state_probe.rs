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

use anyhow::{Result, bail};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use tobii_proto::protocol::{chunk_command, cmd};

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
pub(crate) const EXTENSIONS_ENUMERATE: u32 = 1810;

/// Command 1820 `tracker_enumerate_enabled_extensions` (id at 0x18019d114).
/// The only way to learn which extensions are on, since nothing disables one.
pub(crate) const EXTENSIONS_ENUMERATE_ENABLED: u32 = 1820;

/// Command 1310 `tracker_get_configuration_key` (id at 0x180196456). Takes a
/// key name; the DLL supplies none of its own, so the name must come from the
/// caller.
pub(crate) const CONFIGURATION_KEY: u32 = 1310;

/// Command 3000 `tracker_power_save_activate` (id at 0x18019d803).
pub(crate) const POWER_SAVE_ACTIVATE: u32 = 3000;

/// Command 3010 `tracker_power_save_deactivate` (id at 0x18019fc93).
pub(crate) const POWER_SAVE_DEACTIVATE: u32 = 3010;

/// Command 3040 `tracker_remote_wake_activate` (id at 0x18019d573).
pub(crate) const REMOTE_WAKE_ACTIVATE: u32 = 3040;

/// Command 3050 `tracker_remote_wake_deactivate` (id at 0x1801941a3).
pub(crate) const REMOTE_WAKE_DEACTIVATE: u32 = 3050;

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

    /// The group's commands, the one that restores the resting state first:
    /// the ET5's 1490 reports both states `false` in every answer seen, so
    /// the deactivate is the no-op that shows the answer's shape before the
    /// activate changes anything.
    #[must_use]
    pub(crate) fn steps(self) -> [u32; 2] {
        match self {
            Self::PowerSave => [POWER_SAVE_DEACTIVATE, POWER_SAVE_ACTIVATE],
            Self::RemoteWake => [REMOTE_WAKE_DEACTIVATE, REMOTE_WAKE_ACTIVATE],
        }
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
        /// The command that puts the state back.
        reverts_with: u32,
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
            reverts_with: POWER_SAVE_ACTIVATE,
        },
        arg: Arg::None,
        dll: "0x18019fbc0, id at 0x18019fc93",
    },
    Probe {
        name: "tracker_power_save_activate",
        id: POWER_SAVE_ACTIVATE,
        kind: Kind::Write {
            group: Group::PowerSave,
            reverts_with: POWER_SAVE_DEACTIVATE,
        },
        arg: Arg::None,
        dll: "0x18019d730, id at 0x18019d803",
    },
    Probe {
        name: "tracker_remote_wake_deactivate",
        id: REMOTE_WAKE_DEACTIVATE,
        kind: Kind::Write {
            group: Group::RemoteWake,
            reverts_with: REMOTE_WAKE_ACTIVATE,
        },
        arg: Arg::None,
        dll: "0x1801940d0, id at 0x1801941a3",
    },
    Probe {
        name: "tracker_remote_wake_activate",
        id: REMOTE_WAKE_ACTIVATE,
        kind: Kind::Write {
            group: Group::RemoteWake,
            reverts_with: REMOTE_WAKE_DEACTIVATE,
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
        (Arg::KeyName, Some(k)) => key_name_payload(k),
    };
    let mut writes = chunk_command(id, seq, &payload);
    if writes.len() != 1 {
        bail!("command {id} would need {} bulk writes", writes.len());
    }
    Ok(writes.swap_remove(0))
}

/// What the probe intends to send, in order.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    /// The commands, in the order they would go out. A 1490 is taken before
    /// and after each of them, which [`Plan::render`] shows.
    pub(crate) steps: Vec<u32>,
    /// The user has approved sending the write commands.
    pub(crate) allow_writes: bool,
    /// The key name for command 1310, if it is in the plan.
    pub(crate) config_key: Option<String>,
}

impl Plan {
    /// Every step of the approved groups and reads.
    #[must_use]
    pub(crate) fn for_groups(groups: &[Group], reads: bool) -> Self {
        let mut steps = Vec::new();
        if reads {
            steps.extend(
                PROBES
                    .iter()
                    .filter(|p| p.kind == Kind::Read && p.arg == Arg::None)
                    .map(|p| p.id),
            );
        }
        for g in groups {
            steps.extend_from_slice(&g.steps());
        }
        Self {
            steps,
            allow_writes: !groups.is_empty(),
            config_key: None,
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
        for &id in &self.steps {
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
            if let Kind::Write {
                group,
                reverts_with,
            } = p.kind
            {
                if !self.allow_writes {
                    bail!(
                        "{} ({}) changes tracker state; group {} needs --allow-writes",
                        p.name,
                        id,
                        group.as_str()
                    );
                }
                if !self.steps.contains(&reverts_with) {
                    bail!(
                        "{} ({}) would be left in effect: its revert {reverts_with} is not in \
                         this run",
                        p.name,
                        id
                    );
                }
            }
            if p.arg == Arg::KeyName {
                let Some(key) = self.config_key.as_deref() else {
                    bail!(
                        "command {id} needs a key name (--config-key); the DLL supplies none of \
                         its own, so there is no default to fall back on"
                    );
                };
                if key.is_empty() || key.len() > KEY_NAME_MAX {
                    bail!(
                        "a key name must be 1..={KEY_NAME_MAX} bytes, got {}",
                        key.len()
                    );
                }
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
        let writes: BTreeSet<u32> = self
            .steps
            .iter()
            .copied()
            .filter(|&id| probe(id).is_some_and(|p| !matches!(p.kind, Kind::Read)))
            .collect();
        let _ = writeln!(
            s,
            "state-probe dry run: {} command(s), {} of them writes. Nothing is sent.",
            self.steps.len(),
            writes.len()
        );
        let _ = writeln!(
            s,
            "A 1490 tracker_get_status is taken before and after each step; it is the only \
             command that reports the power-save (index 1) and remote-wake (index 0) state."
        );
        for (seq, &id) in (1u32..).zip(self.steps.iter()) {
            // `check` has already accepted every step.
            let p = probe(id).ok_or_else(|| anyhow::anyhow!("{id} left the whitelist"))?;
            let key = if p.arg == Arg::KeyName {
                self.config_key.as_deref()
            } else {
                None
            };
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
        config_key,
    } = command
    else {
        bail!("not a state-probe command");
    };
    let mut plan = Plan::for_groups(groups, *reads);
    plan.allow_writes = *allow_writes;
    plan.config_key = config_key.clone();
    if config_key.is_some() && !plan.steps.contains(&CONFIGURATION_KEY) {
        plan.steps.push(CONFIGURATION_KEY);
    }
    plan.check()?;
    print!("{}", plan.render()?);
    Ok(())
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
                steps: vec![id],
                allow_writes: true,
                config_key: None,
            };
            let e = plan.check().expect_err("must refuse");
            assert!(e.to_string().contains(needle), "{id}: {e}");
        }
    }

    #[test]
    fn an_id_outside_the_whitelist_is_refused() {
        let plan = Plan {
            steps: vec![1220],
            allow_writes: true,
            config_key: None,
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
            steps: vec![POWER_SAVE_DEACTIVATE, POWER_SAVE_ACTIVATE],
            allow_writes: false,
            config_key: None,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(e.to_string().contains("--allow-writes"), "{e}");
    }

    #[test]
    fn a_write_whose_revert_is_missing_is_refused() {
        let plan = Plan {
            steps: vec![POWER_SAVE_ACTIVATE],
            allow_writes: true,
            config_key: None,
        };
        let e = plan.check().expect_err("must refuse");
        assert!(e.to_string().contains("left in effect"), "{e}");
    }

    #[test]
    fn a_group_brings_both_of_its_commands_with_the_revert_first() {
        assert_eq!(
            Group::PowerSave.steps(),
            [POWER_SAVE_DEACTIVATE, POWER_SAVE_ACTIVATE]
        );
        let plan = Plan::for_groups(&[Group::PowerSave], false);
        plan.check().expect("a whole group passes");
    }

    #[test]
    fn the_configuration_key_needs_a_name_and_bounds_it() {
        let mut plan = Plan {
            steps: vec![CONFIGURATION_KEY],
            allow_writes: false,
            config_key: None,
        };
        let e = plan.check().expect_err("no name");
        assert!(e.to_string().contains("--config-key"), "{e}");
        plan.config_key = Some("x".repeat(KEY_NAME_MAX + 1));
        let e = plan.check().expect_err("too long");
        assert!(e.to_string().contains("must be 1..="), "{e}");
        plan.config_key = Some("serial_number".to_owned());
        plan.check().expect("a short name passes");
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
            config_key: None,
        };
        assert!(plan.check().is_err());
    }

    #[test]
    fn the_reads_plan_holds_no_write_and_passes() {
        let plan = Plan::for_groups(&[], true);
        assert!(!plan.steps.is_empty());
        for &id in &plan.steps {
            assert_eq!(probe(id).expect("whitelisted").kind, Kind::Read, "{id}");
        }
        plan.check().expect("reads alone pass");
    }

    #[test]
    fn the_rendered_plan_shows_the_bytes_and_marks_the_writes() {
        let plan = Plan::for_groups(&[Group::RemoteWake], true);
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
