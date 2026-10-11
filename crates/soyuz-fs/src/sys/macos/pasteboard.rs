//! The general pasteboard (`NSPasteboard`): file URLs plus the paths as text.
//!
//! AppKit's pasteboard may be used from any thread; every call runs inside an
//! autorelease pool so background threads do not leak. Untested on macOS: compile-checked
//! from Linux only.

use std::path::PathBuf;

use objc2::ClassType;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyClass, AnyObject, ProtocolObject};
use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardURLReadingFileURLsOnlyKey, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL};

/// The pasteboard's change count (bumped by every owner change).
pub(crate) fn change_count() -> isize {
    autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount())
}

/// Replace the pasteboard with `paths` as file URLs (one item each) and `text` as the
/// plain-text flavour of the first item. Returns the new change count.
pub(crate) fn write_files(paths: &[PathBuf], text: &str) -> Option<isize> {
    autoreleasepool(|_| {
        let urls: Vec<Retained<NSURL>> = paths.iter().filter_map(NSURL::from_file_path).collect();
        if urls.is_empty() {
            return None;
        }
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        let objs: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            urls.into_iter().map(ProtocolObject::from_retained).collect();
        if !pb.writeObjects(&NSArray::from_retained_slice(&objs)) {
            return None;
        }
        if !text.is_empty()
            && let Some(first) = pb.pasteboardItems().and_then(|items| items.firstObject())
        {
            // SAFETY: `NSPasteboardTypeString` is an immutable AppKit constant.
            let ty = unsafe { NSPasteboardTypeString };
            first.setString_forType(&NSString::from_str(text), ty);
        }
        Some(pb.changeCount())
    })
}

/// File URLs on the pasteboard as local paths (`None` when there are none).
pub(crate) fn read_files() -> Option<Vec<PathBuf>> {
    autoreleasepool(|_| {
        let pb = NSPasteboard::generalPasteboard();
        let class: &AnyClass = NSURL::class();
        let classes = NSArray::from_slice(&[class]);
        let yes = NSNumber::new_bool(true);
        let value: &AnyObject = yes.as_ref();
        // SAFETY: `NSPasteboardURLReadingFileURLsOnlyKey` is an immutable AppKit constant.
        let key = unsafe { NSPasteboardURLReadingFileURLsOnlyKey };
        let options = NSDictionary::from_slices(&[key], &[value]);
        // SAFETY: the class array holds `NSURL` (an `NSPasteboardReading` class) and the
        // options dictionary maps a documented reading-option key to an `NSNumber`, as the
        // method requires.
        let objs = unsafe { pb.readObjectsForClasses_options(&classes, Some(&options)) }?;
        let paths: Vec<PathBuf> = (0..objs.count())
            .map(|i| objs.objectAtIndex(i))
            .filter_map(|o| o.downcast::<NSURL>().ok())
            .filter_map(|u| u.to_file_path())
            .collect();
        (!paths.is_empty()).then_some(paths)
    })
}

/// Empty the pasteboard.
pub(crate) fn clear() {
    autoreleasepool(|_| {
        NSPasteboard::generalPasteboard().clearContents();
    });
}
