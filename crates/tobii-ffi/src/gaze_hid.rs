//! The two gaze-HID entry points of `tobii_internal.h`, and `tobii_open_realm`.
//!
//! The two HID reads are the surprise of the seventeen: **both are real
//! functions for an ET5, not refusals**, which is why their `not_supported!`
//! stubs were wrong (the derivation and every address:
//! `fixtures/analysis/unknown-protocols-2026-10-22/REPORT-A.md` §1, with
//! `hid-realm/REPORT.md` beside it).
//!
//! - `tobii_get_combined_gaze_hid_track_box` turns the property layer's
//!   `TOBII_ERROR_NOT_SUPPORTED` for PRP property 0xf into
//!   `TOBII_ERROR_NO_ERROR` plus a track box the DLL has **hard-coded**, so
//!   not-supported can never leave that entry point.
//! - `tobii_get_gaze_hid_enabled`'s module licence gate is `< 0`, which every
//!   feature group passes, so a consumer ET5 reaches the read and gets
//!   `TOBII_ERROR_NO_ERROR`.
//!
//! `tobii_open_realm` is here for the opposite reason: it is settled on the
//! licence before the DLL contacts the tracker at all, so no realm key, and
//! no part of the exchange behind it, matters to libtobii.

use std::ffi::c_void;

use crate::device::{Device, device_ref};
use crate::status::{
    Status, TOBII_ERROR_INVALID_PARAMETER, TOBII_ERROR_NO_ERROR, TOBII_ERROR_NOT_SUPPORTED,
};
use crate::stub::licence_refused;
use crate::timeouts;
use crate::types::TrackBox;
use tobii_ipc::request::{decode_track_box, kind};

/// The track box the DLL writes for `tobii_get_combined_gaze_hid_track_box`,
/// in millimetres, in `tobii_track_box_t` order: the eight corners it stores
/// as 24 immediates at 0x180149fe7..0x18014a125.
///
/// libtobii answers the tracker's own measured box instead (see the export).
/// These are kept so the difference is on the record: they agree with the
/// ET5's command 1400 answer in y and z exactly, and are 25 mm wider on each
/// side in x — a 300x200 mm front face at 450 mm and a 550x400 mm back face
/// at 900 mm, against the tracker's 250x200 at 450 and 500x400 at 900.
#[cfg(test)]
const DLL_HARD_CODED_TRACK_BOX_MM: [[f32; 3]; 8] = [
    [150.0, 100.0, 450.0],
    [-150.0, 100.0, 450.0],
    [-150.0, -100.0, 450.0],
    [150.0, -100.0, 450.0],
    [275.0, 200.0, 900.0],
    [-275.0, 200.0, 900.0],
    [-275.0, -200.0, 900.0],
    [275.0, -200.0, 900.0],
];

/// The volume in which the tracker's combined gaze is reported over HID, mm.
///
/// **This never answers `TOBII_ERROR_NOT_SUPPORTED`.** The DLL's export
/// (0x180149fb0) reads PRP property 0xf through `tobii_property_get`
/// (0x18015d300, the id at 0x180149fd2), which refuses the property before
/// any request (0x18015d450) because the legacy TTP module publishes module
/// property 1 in none of its lists; and the export then turns **exactly that
/// refusal** into success, filling the output with a box held in its own code
/// and returning `TOBII_ERROR_NO_ERROR` (0x180149fde, the fill
/// 0x180149fe7..0x18014a125, the return at 0x18014a125). Every other result
/// of the property get — a null argument, the callback flag, a request error
/// — passes through unchanged.
///
/// Where libtobii differs, by the user's decision of 2026-10-22: it answers
/// the tracker's **own measured** track box, the answer to command 1400 that
/// tobiid already keeps, rather than the DLL's constants. The two differ only
/// in x, by 25 mm on each side at both planes (see
/// [`DLL_HARD_CODED_TRACK_BOX_MM`]), so an application sees a box 50 mm
/// narrower than on Windows — and sees the same box from this entry point and
/// from `tobii_get_track_box`, where Windows gives two slightly different
/// ones. The DLL asks the tracker nothing here; libtobii does, so this call
/// can also answer a fetch's error, as `tobii_get_track_box` does.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `track_box` must be null or
/// valid for writing one `tobii_track_box_t`.
#[unsafe(no_mangle)]
#[allow(clippy::cast_possible_truncation)] // reason: the C ABI carries float
pub unsafe extern "C" fn tobii_get_combined_gaze_hid_track_box(
    device: *mut Device,
    track_box: *mut TrackBox,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if track_box.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    let b = match d.request(kind::TRACK_BOX, &[], timeouts::FACTS) {
        Ok(p) => match decode_track_box(&p) {
            Some(b) => b,
            None => return d.malformed("combined gaze HID track box"),
        },
        Err(status) => return status,
    };
    let c = b.corners_mm.map(|p| p.map(|v| v as f32));
    let out = TrackBox {
        front_upper_right_xyz: c[0],
        front_upper_left_xyz: c[1],
        front_lower_left_xyz: c[2],
        front_lower_right_xyz: c[3],
        back_upper_right_xyz: c[4],
        back_upper_left_xyz: c[5],
        back_lower_left_xyz: c[6],
        back_lower_right_xyz: c[7],
    };
    // SAFETY: `track_box` is non-null and the caller guarantees it is valid
    // for one `TrackBox`, which has no invalid bit pattern.
    unsafe { track_box.write(out) };
    TOBII_ERROR_NO_ERROR
}

/// The one property value the DLL's module treats as true: it stores the
/// cached 1330 boolean as a byte and tests it for non-zero
/// (`[module+0xe5fe]`, 0x18014da0b), and its 1330 reader makes that byte 1
/// only for the exact string `true`, as `tracker_get_capabilities` does for
/// its own booleans (0x180197622..0x18019763d).
const PROPERTY_TRUE: &str = "true";

/// Whether the tracker reports gaze over HID.
///
/// **This is a real answer for an ET5, not a refusal**, because the module's
/// licence gate here is `< 0` — `cmp [module+0xdac8],0` at 0x18014d851 — which
/// every feature group passes, consumer included. The DLL's export
/// (0x18014d770) checks a null device (`TOBII_ERROR_INVALID_PARAMETER`,
/// unlogged, 0x18014d786), then a null output (`tobii_internal.cpp` line
/// 346), then the callback flag (line 347), then
/// `TOBII_ERROR_NOT_SUPPORTED` with no tracker module of its own (line 354);
/// past the licence gate it answers `TOBII_ERROR_NOT_SUPPORTED` when the
/// cached 1330 answer is missing (`platmod_legacy_ttp.cpp` line 7243) or did
/// not carry property 5 (line 7244), and otherwise writes
/// `[module+0xe5fe] != 0` and returns `TOBII_ERROR_NO_ERROR`
/// (0x18014da0b..0x18014da18).
///
/// The output is **four bytes** (`mov DWORD PTR [rbx],eax` at 0x18014da18),
/// not a `bool`. An ET5's 1330 answer carries property 5 as `false`
/// (`crates/tobii-proto/fixtures/init-rsp-1330.hex`), so the answer is
/// `TOBII_ERROR_NO_ERROR` with zero written.
///
/// libtobii reads the property from tobiid's `DEVICE_INFO`, whose third
/// optional tail carries it, and makes the same distinction the DLL makes: a
/// property the tracker did not report, which is also what a daemon from
/// before that tail sends, answers `TOBII_ERROR_NOT_SUPPORTED`, as the DLL
/// does without the cached property. No licence is checked, as none stands in
/// the way.
///
/// # Safety
/// `device` as `tobii_device_process_callbacks`; `enabled` must be null or
/// valid for writing one `uint32_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tobii_get_gaze_hid_enabled(
    device: *mut Device,
    enabled: *mut u32,
) -> Status {
    // SAFETY: caller guarantees `device` is null or a live handle, not
    // destroyed before this returns.
    let d = match unsafe { device_ref(device) } {
        Ok(d) => d,
        Err(status) => return status,
    };
    if enabled.is_null() {
        return TOBII_ERROR_INVALID_PARAMETER;
    }
    let internal = match d.device_info() {
        Ok(i) => i.internal,
        Err(status) => return status,
    };
    // No tail at all, or a tail whose property 5 is empty: the tracker
    // reported none, which is the DLL's not-supported, not a `false`.
    let Some(internal) = internal.filter(|i| !i.property_5.is_empty()) else {
        return TOBII_ERROR_NOT_SUPPORTED;
    };
    let on = u32::from(internal.property_5 == PROPERTY_TRUE);
    // SAFETY: `enabled` is non-null and the caller guarantees it is valid for
    // one `u32`, which has no invalid bit pattern.
    unsafe { enabled.write(on) };
    TOBII_ERROR_NO_ERROR
}

/// Open a realm on the tracker: `TOBII_ERROR_INSUFFICIENT_LICENSE` for an
/// ET5, with nothing sent to the tracker and the key never read.
///
/// The DLL's export (0x18014c550) checks a null device
/// (`TOBII_ERROR_INVALID_PARAMETER`, unlogged, 0x18014c56b), then a null key
/// (`tobii_internal.cpp` line 559), then a key size of zero or less (line
/// 560), then the callback flag (line 561), and then, with no tracker module
/// of its own, `TOBII_ERROR_NOT_SUPPORTED` (line 568) — behind Tobii's
/// service. On the module, `platmod_ttp_open_realm` (0x1801758c0) tests the
/// licence level as its first instruction (`cmp [rcx+0xdac8],3` at
/// 0x1801758ca) and returns `TOBII_ERROR_INSUFFICIENT_LICENSE` (0x18017593c,
/// `platmod_legacy_ttp.cpp` line 7380) below the internal level. A consumer
/// ET5 is level 0.
///
/// So the DLL never reaches the tracker for an ET5, and whatever the exchange
/// behind that gate needs is beside the point: an application holding a real
/// key gets this same answer from the DLL on Windows. The ET5 does refuse
/// other commands for want of an open realm — command 1310 and stream 0x508
/// answer 0x20000505, naming realm 10000 — but no export of this library can
/// open one, exactly as on Windows.
#[unsafe(no_mangle)]
pub extern "C" fn tobii_open_realm(
    device: *mut Device,
    realm: u32,
    key: *const c_void,
    key_size: u32,
) -> Status {
    let _ = realm;
    // The DLL wants a key and a positive size before its licence check; both
    // are compared only, never read.
    licence_refused("tobii_open_realm", device, key.is_null() || key_size == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::tests::{counting_device, destroy_untouched, fake_daemon};
    use crate::status::{
        TOBII_ERROR_CALLBACK_IN_PROGRESS, TOBII_ERROR_INSUFFICIENT_LICENSE,
        TOBII_ERROR_NOT_SUPPORTED,
    };
    use std::ptr;
    use std::sync::atomic::Ordering;
    use tobii_ipc::request::{self, encode_track_box};

    /// The ET5's own track box, as its command 1400 answers it (the fixture
    /// `crates/tobii-proto/fixtures/init-rsp-1400.hex`, scaled by the
    /// protocol's 1024 units per mm).
    const ET5_TRACK_BOX_MM: [[f64; 3]; 8] = [
        [125.0, 100.0, 450.0],
        [-125.0, 100.0, 450.0],
        [-125.0, -100.0, 450.0],
        [125.0, -100.0, 450.0],
        [250.0, 200.0, 900.0],
        [-250.0, 200.0, 900.0],
        [-250.0, -200.0, 900.0],
        [250.0, -200.0, 900.0],
    ];

    /// A device whose daemon answers `TRACK_BOX` with the ET5's own box.
    fn track_box_device() -> *mut Device {
        let connect = fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![tobii_ipc::encode_subscribed(true)],
            Some(&tobii_ipc::TAG_REQUEST) => {
                let req = request::decode_request(body).expect("request");
                if req.kind == kind::TRACK_BOX {
                    let b = tobii_ipc::geometry::TrackBox {
                        corners_mm: ET5_TRACK_BOX_MM,
                    };
                    vec![tobii_ipc::encode_reply(req.id, 0, &encode_track_box(&b))]
                } else {
                    vec![tobii_ipc::encode_reply(
                        req.id,
                        request::status::NOT_SUPPORTED,
                        &[],
                    )]
                }
            }
            _ => vec![],
        });
        let d = Device::new(connect, 1, 1).expect("device");
        Box::into_raw(Box::new(d))
    }

    #[test]
    #[allow(clippy::cast_possible_truncation)] // reason: the C ABI carries float
    fn the_hid_track_box_is_the_tracker_s_own_and_never_not_supported() {
        let d = track_box_device();
        let mut out = TrackBox::default();
        // SAFETY: `d` is a live handle and `out` is one valid TrackBox.
        let got = unsafe { tobii_get_combined_gaze_hid_track_box(d, &raw mut out) };
        assert_eq!(got, TOBII_ERROR_NO_ERROR, "a real answer, not a refusal");
        assert_ne!(got, TOBII_ERROR_NOT_SUPPORTED);
        let want = ET5_TRACK_BOX_MM.map(|p| p.map(|v| v as f32));
        assert_eq!(
            [
                out.front_upper_right_xyz,
                out.front_upper_left_xyz,
                out.front_lower_left_xyz,
                out.front_lower_right_xyz,
                out.back_upper_right_xyz,
                out.back_upper_left_xyz,
                out.back_lower_left_xyz,
                out.back_lower_right_xyz,
            ],
            want,
            "the tracker's measured box, by the user's decision"
        );
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
    }

    /// Whether two millimetre figures are the same to well inside a micron,
    /// so the comparisons below never hang on a float's exactness.
    fn same_mm(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn the_hid_track_box_differs_from_the_dll_s_constants_only_in_x() {
        // What the divergence costs, pinned so it cannot drift silently: the
        // DLL's constants are 25 mm wider on each side in x, and identical
        // in y and z.
        for (i, (ours, dll)) in ET5_TRACK_BOX_MM
            .iter()
            .zip(DLL_HARD_CODED_TRACK_BOX_MM)
            .enumerate()
        {
            let dll = dll.map(f64::from);
            assert!(
                same_mm((ours[0].abs() - dll[0].abs()).abs(), 25.0),
                "corner {i}: x differs by 25 mm, got {} and {}",
                ours[0],
                dll[0]
            );
            assert!(same_mm(ours[1], dll[1]), "corner {i}: y is the DLL's");
            assert!(same_mm(ours[2], dll[2]), "corner {i}: z is the DLL's");
        }
    }

    #[test]
    fn the_hid_track_box_checks_its_arguments_and_can_answer_a_fetch_error() {
        let mut out = TrackBox::default();
        // SAFETY: a null device is what this entry point must refuse.
        let got = unsafe { tobii_get_combined_gaze_hid_track_box(ptr::null_mut(), &raw mut out) };
        assert_eq!(got, TOBII_ERROR_INVALID_PARAMETER);
        let d = track_box_device();
        // SAFETY: `d` is live; a null output is what this must refuse.
        let got = unsafe { tobii_get_combined_gaze_hid_track_box(d, ptr::null_mut()) };
        assert_eq!(got, TOBII_ERROR_INVALID_PARAMETER);
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };

        // A daemon that does not serve the track box: unlike the DLL, which
        // asks nothing here, this call reports the fetch's error -- and it
        // really does ask, so `destroy_untouched` (which demands that nothing
        // was sent) is the wrong helper here.
        let (nd, frames) = counting_device();
        // SAFETY: `nd` is live and `out` is one valid TrackBox.
        let got = unsafe { tobii_get_combined_gaze_hid_track_box(nd, &raw mut out) };
        assert_ne!(got, TOBII_ERROR_NO_ERROR, "a failed fetch is reported");
        assert_eq!(
            frames.load(Ordering::Relaxed),
            1,
            "it asked the daemon once"
        );
        // SAFETY: `nd` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(nd) };
    }

    /// A device whose daemon answers `DEVICE_INFO` with `internal` holding
    /// `property_5`, or with no internal tail at all when `tail` is `None`.
    fn hid_device(tail: Option<&str>) -> *mut Device {
        let internal = tail.map(|p| request::DeviceInfoInternal {
            platform_type: "IS5LEYETRACKER5".into(),
            property_4: String::new(),
            property_5: p.into(),
        });
        let payload = request::encode_device_info(&request::DeviceInfo {
            serial_number: "x".into(),
            model: "IS5_Large_Eyetracker_5".into(),
            generation: "IS5".into(),
            firmware_version: "f".into(),
            integration_type: "Peripheral".into(),
            internal,
        });
        let connect = fake_daemon(move |body| match body.first() {
            Some(&tobii_ipc::TAG_SUBSCRIBE) => vec![tobii_ipc::encode_subscribed(true)],
            Some(&tobii_ipc::TAG_REQUEST) => {
                let req = request::decode_request(body).expect("request");
                if req.kind == kind::DEVICE_INFO {
                    vec![tobii_ipc::encode_reply(req.id, 0, &payload)]
                } else {
                    vec![tobii_ipc::encode_reply(
                        req.id,
                        request::status::NOT_SUPPORTED,
                        &[],
                    )]
                }
            }
            _ => vec![],
        });
        let d = Device::new(connect, 1, 1).expect("device");
        Box::into_raw(Box::new(d))
    }

    /// What the export answers for a tracker whose 1330 reported `tail`.
    fn hid_enabled(tail: Option<&str>) -> (Status, u32) {
        let d = hid_device(tail);
        let mut out = u32::MAX;
        // SAFETY: `d` is a live handle and `out` is one valid u32.
        let got = unsafe { tobii_get_gaze_hid_enabled(d, &raw mut out) };
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
        (got, out)
    }

    #[test]
    fn gaze_hid_enabled_answers_the_tracker_s_own_property_5() {
        // The ET5 reports `false`, so this is its real answer: no error and a
        // four-byte zero, not a refusal.
        assert_eq!(hid_enabled(Some("false")), (TOBII_ERROR_NO_ERROR, 0));
        // A tracker that said `true` would get a one; only that exact string
        // counts, as the DLL's 1330 reader has it.
        assert_eq!(hid_enabled(Some("true")), (TOBII_ERROR_NO_ERROR, 1));
        assert_eq!(hid_enabled(Some("True")), (TOBII_ERROR_NO_ERROR, 0));
        assert_eq!(hid_enabled(Some("yes")), (TOBII_ERROR_NO_ERROR, 0));
    }

    #[test]
    fn a_property_the_tracker_did_not_report_is_not_supported() {
        // Empty is what a tracker that sent no property 5 gives, and what a
        // daemon from before the tail gives: the DLL's answer without its
        // cached property, not a `false`.
        let (status, out) = hid_enabled(Some(""));
        assert_eq!(status, TOBII_ERROR_NOT_SUPPORTED);
        assert_eq!(out, u32::MAX, "nothing is written on a refusal");
        // No internal tail at all, as from an older daemon still.
        let (status, out) = hid_enabled(None);
        assert_eq!(status, TOBII_ERROR_NOT_SUPPORTED);
        assert_eq!(out, u32::MAX);
    }

    #[test]
    fn gaze_hid_enabled_checks_its_arguments_and_reports_a_fetch_error() {
        let mut out = u32::MAX;
        // SAFETY: a null device is what this entry point must refuse.
        let got = unsafe { tobii_get_gaze_hid_enabled(ptr::null_mut(), &raw mut out) };
        assert_eq!(got, TOBII_ERROR_INVALID_PARAMETER);
        let d = hid_device(Some("false"));
        // SAFETY: `d` is live; a null output is what this must refuse.
        let got = unsafe { tobii_get_gaze_hid_enabled(d, ptr::null_mut()) };
        assert_eq!(got, TOBII_ERROR_INVALID_PARAMETER);
        crate::device::call(|| {
            // SAFETY: `d` is live and `out` is one valid u32.
            let got = unsafe { tobii_get_gaze_hid_enabled(d, &raw mut out) };
            assert_eq!(got, TOBII_ERROR_CALLBACK_IN_PROGRESS);
        });
        // SAFETY: `d` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(d) };
        assert_eq!(out, u32::MAX, "nothing written on any refusal");

        // A daemon that does not serve the device info: the fetch's error.
        let (nd, frames) = counting_device();
        // SAFETY: `nd` is live and `out` is one valid u32.
        let got = unsafe { tobii_get_gaze_hid_enabled(nd, &raw mut out) };
        assert_ne!(got, TOBII_ERROR_NO_ERROR, "a failed fetch is reported");
        assert_eq!(
            frames.load(Ordering::Relaxed),
            1,
            "it asked the daemon once"
        );
        // SAFETY: `nd` is a live handle this test owns and no longer uses.
        unsafe { crate::api::tobii_device_destroy(nd) };
    }

    #[test]
    fn open_realm_is_refused_on_the_licence_without_reading_the_key() {
        let (d, frames) = counting_device();
        let key = [0u8; 16];
        assert_eq!(
            tobii_open_realm(d, 10_000, key.as_ptr().cast(), 16),
            TOBII_ERROR_INSUFFICIENT_LICENSE,
            "as the DLL answers a consumer ET5, before any tracker contact"
        );
        // The DLL wants both before its licence check.
        assert_eq!(
            tobii_open_realm(d, 10_000, ptr::null(), 16),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_open_realm(d, 10_000, key.as_ptr().cast(), 0),
            TOBII_ERROR_INVALID_PARAMETER
        );
        assert_eq!(
            tobii_open_realm(ptr::null_mut(), 10_000, key.as_ptr().cast(), 16),
            TOBII_ERROR_INVALID_PARAMETER
        );
        crate::device::call(|| {
            assert_eq!(
                tobii_open_realm(d, 10_000, key.as_ptr().cast(), 16),
                TOBII_ERROR_CALLBACK_IN_PROGRESS
            );
        });
        assert_eq!(frames.load(Ordering::Relaxed), 0, "no daemon round trip");
        destroy_untouched(d, &frames);
    }
}
