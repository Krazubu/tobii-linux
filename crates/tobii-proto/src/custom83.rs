//! The 0x1770 `algodbg` stream, the Stream Engine's custom stream 0, read as
//! the Stream Engine reads it.
//!
//! The DLL's tracker module starts 0x1770 for a subscriber to custom stream 0
//! (`tobii_custom_stream_subscribe`; command 1220 from 0x180160810) and hands
//! the subscriber each message's key 1. The tracker's column list for the
//! stream (its 1210 answer) names key 1 "Debug data" and no other key. While
//! the stream runs the ET5 sends the 70 bytes of
//! `fixtures/algodbg-frame-1770.hex` over and over, key 1 an empty blob in
//! each of them (stream-probe, 2026-10-13).
//!
//! The DLL reads such a message with its stream parser's own case for 0x1770
//! (0x18017cf63), and drops the message, with a log line and no callback, at
//! the first read that fails:
//!
//! - the header, the field id `(n << 16) | 0x0bb8`, gives the number of
//!   fields n (0x180004a40, as [`keyed_count`] reads it);
//! - each of the n fields is `05 0x00020bb9` and a type-2 key (0x1800049f0);
//! - key 1's value is read as a blob, type 0x15 (0x180003790), so a key 1 of
//!   any other type drops the message;
//! - for any other key no value is read (0x18017cff0 goes on to the next
//!   field), so the next field's `05 0x00020bb9` has to follow the key: a
//!   value there drops the message, unless the key is the last of the n;
//! - nothing after the n fields is read.
//!
//! `process_custom_stream` (0x180189920) then hands the subscriber key 1's
//! bytes, the last key 1's when there are several (its loop at 0x180189980
//! goes through every field), or no bytes when there is no key 1: it calls
//! back either way. [`decode_custom_frame`] reads a message the same way,
//! but for a blob whose two lengths disagree, which the tracker never sends:
//! the DLL's blob reader takes the length in front of the bytes and does not
//! check the TLV's own, where the decoder takes the bytes within the TLV
//! ([`Tlv::bytes`](crate::tlv::Tlv::bytes)).

use crate::protocol::{Message, STREAM_ID_ALGODBG};
use crate::tlv::{KEY_FIELD_ID, keyed_count};

/// Key 1, "Debug data", a blob: the one key the tracker lists for the stream
/// and the one the DLL reads.
pub const KEY_DEBUG_DATA: u32 = 1;

/// A 0x1770 message, as the DLL hands it to custom stream 0's subscriber.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomFrame {
    /// Key 1's bytes. Empty when its blob is, as in every message of the
    /// ET5, or when the message has no key 1: the DLL calls back with a size
    /// of 0 for both.
    pub data: Vec<u8>,
}

/// Decode a 0x1770 message as the DLL reads it (see [`crate::custom83`]);
/// `None` for any other message and for one the DLL drops.
#[must_use]
pub fn decode_custom_frame(msg: &Message<'_>) -> Option<CustomFrame> {
    if msg.id != STREAM_ID_ALGODBG {
        return None;
    }
    let mut entries = msg.tlvs();
    let mut data: &[u8] = &[];
    for _ in 0..keyed_count(&mut entries)? {
        if entries.next()?.field_id()? != KEY_FIELD_ID {
            return None;
        }
        // The DLL reads a value for key 1 only.
        if entries.next()?.typed_u32()? == KEY_DEBUG_DATA {
            data = entries.next()?.bytes()?;
        }
    }
    Some(CustomFrame {
        data: data.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{HEADER_LEN, MARKER_STREAM, STREAM_ID_GAZE, parse_message};
    use crate::tlv::{FIELD_KEYED_HEADER, TYPE_STRING, TYPE_U32_ALT, TlvWriter};

    /// Where the ET5's message puts its TLVs, after the 32-byte header and
    /// `00 00`: the header, then the field's id, key and blob, 9 bytes each.
    const HEADER: usize = HEADER_LEN + 2;
    const FIELD: usize = HEADER + 9;
    const KEY: usize = FIELD + 9;
    const BLOB: usize = KEY + 9;

    /// The ET5's 0x1770 message.
    fn et5() -> Vec<u8> {
        crate::fixture!("algodbg-frame-1770")
    }

    /// The data of `bytes` decoded, `None` for no frame.
    fn data(bytes: &[u8]) -> Option<Vec<u8>> {
        decode_custom_frame(&parse_message(bytes).expect("message")).map(|f| f.data)
    }

    /// The data of the ET5's message with `edit` applied (offsets are into
    /// the message, header included).
    fn edited(edit: impl FnOnce(&mut Vec<u8>)) -> Option<Vec<u8>> {
        let mut bytes = et5();
        edit(&mut bytes);
        data(&bytes)
    }

    /// The data of the ET5's message with `payload` in place of its own.
    fn data_in(payload: &[u8]) -> Option<Vec<u8>> {
        let len = |n: usize| u32::try_from(n).expect("fits");
        edited(|b| {
            b.truncate(HEADER_LEN);
            b.extend_from_slice(payload);
            let total = len(b.len());
            b[4..8].copy_from_slice(&total.to_le_bytes());
            b[28..32].copy_from_slice(&len(payload.len()).to_be_bytes());
        })
    }

    /// A payload whose header announces `n` keyed fields, then what `fields`
    /// writes.
    fn keyed(n: u32, fields: impl FnOnce(&mut TlvWriter)) -> Vec<u8> {
        let mut w = TlvWriter::new();
        w.field_id((n << 16) | FIELD_KEYED_HEADER);
        fields(&mut w);
        w.finish()
    }

    /// Announces keyed field `key`; its value, if it has one, follows.
    fn announce(w: &mut TlvWriter, key: u32) -> &mut TlvWriter {
        w.field_id(KEY_FIELD_ID).u32(key)
    }

    #[test]
    fn the_et5s_message_is_key_1_with_an_empty_blob() {
        let bytes = et5();
        let msg = parse_message(&bytes).expect("message");
        assert_eq!(bytes.len(), 70);
        assert_eq!((msg.marker, msg.id), (MARKER_STREAM, STREAM_ID_ALGODBG));

        let frame = decode_custom_frame(&msg);

        assert_eq!(frame, Some(CustomFrame { data: vec![] }));
        assert_eq!(
            msg.payload,
            keyed(1, |w| {
                announce(w, KEY_DEBUG_DATA).bytes(&[]);
            })
        );
    }

    #[test]
    fn key_1s_bytes_are_the_frames_data() {
        let three = keyed(1, |w| {
            announce(w, KEY_DEBUG_DATA).bytes(&[1, 2, 3]);
        });

        assert_eq!(data_in(&three), Some(vec![1, 2, 3]));
    }

    /// The DLL reads key 1 with its blob reader (0x180003790), which takes
    /// type 0x15 alone.
    #[test]
    fn a_key_1_that_is_not_a_blob_drops_the_message() {
        let word = keyed(1, |w| {
            announce(w, KEY_DEBUG_DATA).u32(0);
        });

        assert_eq!(data_in(&word), None, "a u32");
        assert_eq!(edited(|b| b[BLOB] = TYPE_STRING), None, "a string");
    }

    /// Without key 1, `process_custom_stream` still calls back, with a size
    /// of 0.
    #[test]
    fn a_message_without_key_1_is_a_frame_with_no_data() {
        // Its key announced as 2, the ET5's blob is the value of a last
        // field whose value is not read.
        assert_eq!(edited(|b| b[BLOB - 1] = 2), Some(vec![]));
        assert_eq!(data_in(&keyed(0, |_| {})), Some(vec![]), "no field");
    }

    #[test]
    fn another_streams_message_is_not_a_custom_frame() {
        // 0x1774 is the stream the catalogue names custom.
        for id in [STREAM_ID_GAZE, 0x1774] {
            let other = edited(|b| b[20..24].copy_from_slice(&id.to_be_bytes()));
            assert_eq!(other, None, "{id:#x}");
        }
        assert_eq!(data(&crate::fixture!("session1-gaze-frame")), None);
    }

    #[test]
    fn a_message_cut_short_is_dropped() {
        let bytes = et5();

        for len in HEADER_LEN..bytes.len() {
            assert_eq!(data(&bytes[..len]), None, "{len} bytes");
        }
    }

    /// The DLL reads no value for a key other than 1, so the next field's
    /// header has to follow the key at once: a value there fails the read of
    /// that header and drops the message, and a value after the last field
    /// is never read.
    #[test]
    fn another_key_has_no_value_as_the_dll_reads_it() {
        let point = [1.0, 2.0, 3.0];
        let debug = |w: &mut TlvWriter| {
            announce(w, KEY_DEBUG_DATA).bytes(&[7, 8, 9]);
        };
        let bare_first = keyed(2, |w| {
            announce(w, 2);
            debug(w);
        });
        let point_first = keyed(2, |w| {
            announce(w, 2).point_mm(point);
            debug(w);
        });
        let word_first = keyed(2, |w| {
            announce(w, 2).u32(5);
            debug(w);
        });
        let point_last = keyed(2, |w| {
            debug(w);
            announce(w, 2).point_mm(point);
        });

        assert_eq!(data_in(&bare_first), Some(vec![7, 8, 9]));
        assert_eq!(data_in(&point_first), None);
        assert_eq!(data_in(&word_first), None);
        assert_eq!(data_in(&point_last), Some(vec![7, 8, 9]));
    }

    #[test]
    fn the_header_says_how_many_fields_are_read() {
        let two_blobs = |n: u32| {
            data_in(&keyed(n, |w| {
                announce(w, KEY_DEBUG_DATA).bytes(&[1, 2]);
                announce(w, KEY_DEBUG_DATA).bytes(&[3]);
            }))
        };

        // process_custom_stream's loop (0x180189980) keeps the last key 1.
        assert_eq!(two_blobs(2), Some(vec![3]));
        assert_eq!(two_blobs(1), Some(vec![1, 2]), "the second is not read");
        assert_eq!(two_blobs(3), None, "a field more than the message holds");
    }

    /// The header's field id ends in 0x0bb8 (0x180004a40), and each field is
    /// `05 0x00020bb9`, the whole word compared, and a type-2 key
    /// (0x1800049f0).
    #[test]
    fn a_header_or_field_announced_otherwise_drops_the_message() {
        assert_eq!(edited(|b| b[HEADER + 8] = 0xb9), None, "0x0001_0bb9");
        assert_eq!(edited(|b| b[FIELD + 8] = 0xb8), None, "0x0002_0bb8");
        assert_eq!(edited(|b| b[FIELD + 6] = 3), None, "0x0003_0bb9");
        assert_eq!(edited(|b| b[KEY] = TYPE_U32_ALT), None, "a key of type 1");
    }
}
