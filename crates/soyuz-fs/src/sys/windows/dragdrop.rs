//! OLE drag source for dragging files out of Soyuz (ole32 `DoDragDrop`).
//!
//! The COM objects are written by hand with `#[repr(C)]` vtables (windows-sys has no
//! interface types): an `IDataObject` serving `HGLOBAL` blocks prepared by the caller,
//! its `IEnumFORMATETC`, and an `IDropSource`. No shell API is involved (no shell data
//! object, shell drag loop or drag-image helper): targets get plain clipboard formats and
//! the system's default drag cursors.
//!
//! Formats a target writes back with `SetData` ("Performed DropEffect", "Paste
//! Succeeded", "Logical Performed DropEffect", ...) are kept so the caller can tell an
//! optimized move (the target moved the files) from an unoptimized one (the source has
//! to delete).

use std::ffi::c_void;
use std::io;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, DV_E_FORMATETC, DV_E_TYMED,
    E_INVALIDARG, E_NOINTERFACE, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, GlobalFree, HGLOBAL, OLE_E_ADVISENOTSUPPORTED,
    POINT, S_FALSE, S_OK,
};
use windows_sys::Win32::System::Com::{DATADIR_GET, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL};
use windows_sys::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::{DoDragDrop, OleInitialize, OleUninitialize, ReleaseStgMedium};
use windows_sys::Win32::System::SystemServices::MK_LBUTTON;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GetAncestor, GetCursorPos, GetSystemMetrics, SM_SWAPBUTTON, WindowFromPoint,
};
use windows_sys::core::{BOOL, GUID, HRESULT};

/// Register (or look up) a named clipboard format. 0 on failure.
pub(crate) fn register_format(name: &str) -> u16 {
    let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: `w` is a NUL-terminated UTF-16 string that outlives the call.
    let id = unsafe { RegisterClipboardFormatW(w.as_ptr()) };
    u16::try_from(id).unwrap_or(0)
}

/// The primary (logical left) mouse button is physically down right now.
pub(crate) fn primary_button_down() -> bool {
    // SAFETY: plain metric / key-state queries.
    unsafe {
        let vk = if GetSystemMetrics(SM_SWAPBUTTON) != 0 { VK_RBUTTON } else { VK_LBUTTON };
        GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0
    }
}

/// How the OLE drag ended.
pub(crate) struct DragResult {
    /// `DRAGDROP_S_DROP` happened.
    pub dropped: bool,
    /// The pointer came back over `return_hwnd`: the drag was cancelled to resume the
    /// app's own drag.
    pub returned: bool,
    /// The effect `DoDragDrop` reported.
    pub effect: u32,
    /// Formats the target wrote back with `SetData` (`(format id, bytes)`).
    pub written: Vec<(u16, Vec<u8>)>,
}

/// Run a modal OLE drag of `formats` (`(clipboard format id, bytes)`) on the calling UI
/// thread. `return_hwnd` (0 = none): moving back over this top-level window cancels the
/// drag with `returned = true`.
pub(crate) fn do_drag(formats: Vec<(u16, Vec<u8>)>, allowed: u32, return_hwnd: isize) -> io::Result<DragResult> {
    // SAFETY: OLE init on the calling thread; balanced below on success (S_OK / S_FALSE).
    let hr = unsafe { OleInitialize(ptr::null()) };
    if hr < 0 {
        return Err(io::Error::other(format!("OleInitialize failed: {hr:#x}")));
    }
    let data = DataObject::new(formats);
    let source = DropSource::new(return_hwnd);
    let mut effect = 0u32;
    // SAFETY: both pointers are live COM objects we hold a reference to for the whole
    // call; `effect` outlives it. `DoDragDrop` runs a modal loop on this thread.
    let hr = unsafe { DoDragDrop(data.cast(), source.cast(), allowed, &mut effect) };
    // SAFETY: we still own one reference each (targets may keep their own).
    let (written, returned) = unsafe {
        let written = (*data).written.lock().map(|w| w.clone()).unwrap_or_default();
        let returned = (*source).returned.load(Ordering::SeqCst);
        release(data.cast());
        release(source.cast());
        (written, returned)
    };
    // SAFETY: balances the successful OleInitialize above.
    unsafe { OleUninitialize() };
    match hr {
        DRAGDROP_S_DROP => Ok(DragResult { dropped: true, returned: false, effect, written }),
        DRAGDROP_S_CANCEL => Ok(DragResult { dropped: false, returned, effect: 0, written }),
        e => Err(io::Error::other(format!("DoDragDrop failed: {e:#x}"))),
    }
}

/// Diagnostics / tests: build the data object and read every format back through its
/// vtable (`EnumFormatEtc` + `QueryGetData` + `GetData`), plus a `SetData` round trip of
/// `write_back`. Returns `(enumerated formats with their bytes, value read back for
/// write_back)`.
#[allow(clippy::type_complexity)]
pub(crate) fn read_back(
    formats: Vec<(u16, Vec<u8>)>,
    write_back: Option<(u16, Vec<u8>)>,
) -> io::Result<(Vec<(u16, Vec<u8>)>, Option<Vec<u8>>)> {
    let obj = DataObject::new(formats);
    let this = obj.cast::<c_void>();
    // SAFETY: `obj` is a live data object (refcount 1) used only through its vtable,
    // like a COM client would; every medium returned is released with
    // ReleaseStgMedium, the enumerator and the object are released at the end.
    unsafe {
        let vt = &*(*obj).vtbl;
        let mut out = Vec::new();
        let mut en: *mut c_void = ptr::null_mut();
        let hr = (vt.enum_format_etc)(this, DATADIR_GET as u32, &mut en);
        if hr != S_OK || en.is_null() {
            release(this);
            return Err(io::Error::other(format!("EnumFormatEtc failed: {hr:#x}")));
        }
        let evt = &**(en as *mut *const EnumVtbl);
        loop {
            let mut fe = FORMATETC::default();
            let mut got = 0u32;
            if (evt.next)(en, 1, &mut fe, &mut got) != S_OK || got != 1 {
                break;
            }
            if (vt.query_get_data)(this, &fe) != S_OK {
                release(en);
                release(this);
                return Err(io::Error::other(format!("QueryGetData refused format {}", fe.cfFormat)));
            }
            let mut med: STGMEDIUM = std::mem::zeroed();
            let hr = (vt.get_data)(this, &fe, &mut med);
            if hr != S_OK {
                release(en);
                release(this);
                return Err(io::Error::other(format!("GetData({}) failed: {hr:#x}", fe.cfFormat)));
            }
            out.push((fe.cfFormat, global_bytes(med.u.hGlobal)));
            ReleaseStgMedium(&mut med);
        }
        release(en);
        let mut echoed = None;
        if let Some((cf, bytes)) = write_back {
            let fe = formatetc(cf);
            let mut med: STGMEDIUM = std::mem::zeroed();
            med.tymed = TYMED_HGLOBAL as u32;
            med.u.hGlobal = global_from(&bytes);
            // fRelease = TRUE: the object owns (and frees) the block on success.
            if (vt.set_data)(this, &fe, &mut med, 1) != S_OK {
                ReleaseStgMedium(&mut med);
            }
            let mut med: STGMEDIUM = std::mem::zeroed();
            if (vt.get_data)(this, &fe, &mut med) == S_OK {
                echoed = Some(global_bytes(med.u.hGlobal));
                ReleaseStgMedium(&mut med);
            }
        }
        release(this);
        Ok((out, echoed))
    }
}

// ---------------------------------------------------------------------------------
// COM plumbing

const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IDATAOBJECT: GUID = GUID::from_u128(0x0000010e_0000_0000_c000_000000000046);
const IID_IDROPSOURCE: GUID = GUID::from_u128(0x00000121_0000_0000_c000_000000000046);
const IID_IENUMFORMATETC: GUID = GUID::from_u128(0x00000103_0000_0000_c000_000000000046);

fn guid_eq(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}

/// `IUnknown::Release` through the object's vtable.
///
/// # Safety
/// `this` must be a live COM object.
unsafe fn release(this: *mut c_void) {
    // SAFETY: every COM object starts with a pointer to a vtable whose third slot is Release.
    unsafe {
        let vt = *(this as *mut *const UnknownVtbl);
        ((*vt).release)(this);
    }
}

#[repr(C)]
struct UnknownVtbl {
    query_interface: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
    release: unsafe extern "system" fn(*mut c_void) -> u32,
}

fn formatetc(cf: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: cf,
        ptd: ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT,
        lindex: -1,
        tymed: TYMED_HGLOBAL as u32,
    }
}

/// Copy `bytes` into a new movable global block (null on failure).
fn global_from(bytes: &[u8]) -> HGLOBAL {
    // SAFETY: the block has at least `bytes.len()` bytes; it is locked, filled with
    // exactly that many bytes and unlocked; freed on lock failure.
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if h.is_null() {
            return h;
        }
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            GlobalFree(h);
            return ptr::null_mut();
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        GlobalUnlock(h);
        h
    }
}

/// Bytes of a global block (empty when it cannot be locked).
///
/// # Safety
/// `h` must be a valid global memory handle (or null).
unsafe fn global_bytes(h: HGLOBAL) -> Vec<u8> {
    if h.is_null() {
        return Vec::new();
    }
    // SAFETY: lock, copy `GlobalSize` bytes, unlock; no pointer is kept.
    unsafe {
        let size = GlobalSize(h);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return Vec::new();
        }
        let v = std::slice::from_raw_parts(p, size).to_vec();
        GlobalUnlock(h);
        v
    }
}

// --- IDataObject -------------------------------------------------------------------

#[repr(C)]
struct DataObjectVtbl {
    base: UnknownVtbl,
    get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    get_data_here: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM) -> HRESULT,
    query_get_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC) -> HRESULT,
    get_canonical_format_etc: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut FORMATETC) -> HRESULT,
    set_data: unsafe extern "system" fn(*mut c_void, *const FORMATETC, *mut STGMEDIUM, BOOL) -> HRESULT,
    enum_format_etc: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    d_advise: unsafe extern "system" fn(*mut c_void, *const FORMATETC, u32, *mut c_void, *mut u32) -> HRESULT,
    d_unadvise: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    enum_d_advise: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct DataObject {
    vtbl: *const DataObjectVtbl,
    refs: AtomicU32,
    /// Offered formats, in preference order.
    formats: Vec<(u16, Vec<u8>)>,
    /// Formats written by the target (`SetData`).
    written: Mutex<Vec<(u16, Vec<u8>)>>,
}

static DATA_OBJECT_VTBL: DataObjectVtbl = DataObjectVtbl {
    base: UnknownVtbl { query_interface: dobj_qi, add_ref: dobj_add_ref, release: dobj_release },
    get_data: dobj_get_data,
    get_data_here: dobj_get_data_here,
    query_get_data: dobj_query_get_data,
    get_canonical_format_etc: dobj_canonical,
    set_data: dobj_set_data,
    enum_format_etc: dobj_enum,
    d_advise: dobj_dadvise,
    d_unadvise: dobj_dunadvise,
    enum_d_advise: dobj_enum_dadvise,
};

impl DataObject {
    fn new(formats: Vec<(u16, Vec<u8>)>) -> *mut DataObject {
        Box::into_raw(Box::new(DataObject {
            vtbl: &DATA_OBJECT_VTBL,
            refs: AtomicU32::new(1),
            formats,
            written: Mutex::new(Vec::new()),
        }))
    }

    /// Bytes for a request, or the HRESULT refusing it.
    fn lookup(&self, fe: &FORMATETC) -> Result<Vec<u8>, HRESULT> {
        if fe.dwAspect != DVASPECT_CONTENT {
            return Err(DV_E_FORMATETC);
        }
        let found =
            self.formats.iter().find(|(cf, _)| *cf == fe.cfFormat).map(|(_, b)| b.clone()).or_else(|| {
                self.written.lock().ok()?.iter().find(|(cf, _)| *cf == fe.cfFormat).map(|(_, b)| b.clone())
            });
        let Some(bytes) = found else { return Err(DV_E_FORMATETC) };
        if fe.tymed & TYMED_HGLOBAL as u32 == 0 {
            return Err(DV_E_TYMED);
        }
        Ok(bytes)
    }
}

unsafe extern "system" fn dobj_qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: pointers checked above; `this` is our object.
    unsafe {
        if guid_eq(&*riid, &IID_IUNKNOWN) || guid_eq(&*riid, &IID_IDATAOBJECT) {
            dobj_add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn dobj_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live DataObject.
    unsafe { (*(this as *const DataObject)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn dobj_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live DataObject; the last reference frees the box.
    unsafe {
        let left = (*(this as *const DataObject)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if left == 0 {
            drop(Box::from_raw(this as *mut DataObject));
        }
        left
    }
}

unsafe extern "system" fn dobj_get_data(this: *mut c_void, fe: *const FORMATETC, med: *mut STGMEDIUM) -> HRESULT {
    if fe.is_null() || med.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: pointers checked; `this` is our object; the caller owns `med` afterwards
    // and frees the block with ReleaseStgMedium (pUnkForRelease = null).
    unsafe {
        let obj = &*(this as *const DataObject);
        match obj.lookup(&*fe) {
            Ok(bytes) => {
                let h = global_from(&bytes);
                if h.is_null() {
                    return E_OUTOFMEMORY;
                }
                (*med).tymed = TYMED_HGLOBAL as u32;
                (*med).u.hGlobal = h;
                (*med).pUnkForRelease = ptr::null_mut();
                S_OK
            }
            Err(hr) => hr,
        }
    }
}

unsafe extern "system" fn dobj_get_data_here(_: *mut c_void, _: *const FORMATETC, _: *mut STGMEDIUM) -> HRESULT {
    E_NOTIMPL
}

unsafe extern "system" fn dobj_query_get_data(this: *mut c_void, fe: *const FORMATETC) -> HRESULT {
    if fe.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: pointer checked; `this` is our object.
    unsafe {
        match (*(this as *const DataObject)).lookup(&*fe) {
            Ok(_) => S_OK,
            Err(hr) => hr,
        }
    }
}

unsafe extern "system" fn dobj_canonical(_: *mut c_void, fe_in: *const FORMATETC, fe_out: *mut FORMATETC) -> HRESULT {
    if fe_in.is_null() || fe_out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: pointers checked.
    unsafe {
        *fe_out = *fe_in;
        (*fe_out).ptd = ptr::null_mut();
    }
    DATA_S_SAMEFORMATETC
}

unsafe extern "system" fn dobj_set_data(
    this: *mut c_void,
    fe: *const FORMATETC,
    med: *mut STGMEDIUM,
    release_it: BOOL,
) -> HRESULT {
    if fe.is_null() || med.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: pointers checked; `this` is our object. With fRelease the medium is ours
    // on success, so it is released after copying; on failure the caller keeps it.
    unsafe {
        let obj = &*(this as *const DataObject);
        let fe = &*fe;
        if (*med).tymed != TYMED_HGLOBAL as u32 || fe.tymed & TYMED_HGLOBAL as u32 == 0 {
            return E_NOTIMPL;
        }
        let bytes = global_bytes((*med).u.hGlobal);
        if let Ok(mut w) = obj.written.lock() {
            w.retain(|(cf, _)| *cf != fe.cfFormat);
            w.push((fe.cfFormat, bytes));
        }
        if release_it != 0 {
            ReleaseStgMedium(med);
        }
        S_OK
    }
}

unsafe extern "system" fn dobj_enum(this: *mut c_void, dir: u32, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: pointer checked; `this` is our object.
    unsafe {
        *out = ptr::null_mut();
        if dir != DATADIR_GET as u32 {
            return E_NOTIMPL;
        }
        let obj = &*(this as *const DataObject);
        let items: Vec<FORMATETC> = obj.formats.iter().map(|(cf, _)| formatetc(*cf)).collect();
        *out = FormatEnum::new(items, 0).cast();
    }
    S_OK
}

unsafe extern "system" fn dobj_dadvise(
    _: *mut c_void,
    _: *const FORMATETC,
    _: u32,
    _: *mut c_void,
    _: *mut u32,
) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn dobj_dunadvise(_: *mut c_void, _: u32) -> HRESULT {
    OLE_E_ADVISENOTSUPPORTED
}

unsafe extern "system" fn dobj_enum_dadvise(_: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if !out.is_null() {
        // SAFETY: pointer checked.
        unsafe { *out = ptr::null_mut() };
    }
    OLE_E_ADVISENOTSUPPORTED
}

// --- IEnumFORMATETC ----------------------------------------------------------------

#[repr(C)]
struct EnumVtbl {
    base: UnknownVtbl,
    next: unsafe extern "system" fn(*mut c_void, u32, *mut FORMATETC, *mut u32) -> HRESULT,
    skip: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    clone: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct FormatEnum {
    vtbl: *const EnumVtbl,
    refs: AtomicU32,
    items: Vec<FORMATETC>,
    pos: AtomicUsize,
}

static ENUM_VTBL: EnumVtbl = EnumVtbl {
    base: UnknownVtbl { query_interface: enum_qi, add_ref: enum_add_ref, release: enum_release },
    next: enum_next,
    skip: enum_skip,
    reset: enum_reset,
    clone: enum_clone,
};

impl FormatEnum {
    fn new(items: Vec<FORMATETC>, pos: usize) -> *mut FormatEnum {
        Box::into_raw(Box::new(FormatEnum {
            vtbl: &ENUM_VTBL,
            refs: AtomicU32::new(1),
            items,
            pos: AtomicUsize::new(pos),
        }))
    }
}

unsafe extern "system" fn enum_qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: pointers checked; `this` is our enumerator.
    unsafe {
        if guid_eq(&*riid, &IID_IUNKNOWN) || guid_eq(&*riid, &IID_IENUMFORMATETC) {
            enum_add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn enum_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live FormatEnum.
    unsafe { (*(this as *const FormatEnum)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn enum_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live FormatEnum; the last reference frees the box.
    unsafe {
        let left = (*(this as *const FormatEnum)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if left == 0 {
            drop(Box::from_raw(this as *mut FormatEnum));
        }
        left
    }
}

unsafe extern "system" fn enum_next(this: *mut c_void, celt: u32, out: *mut FORMATETC, fetched: *mut u32) -> HRESULT {
    if out.is_null() || (celt > 1 && fetched.is_null()) {
        return E_POINTER;
    }
    // SAFETY: `out` holds room for `celt` elements (caller contract); `this` is ours.
    unsafe {
        let e = &*(this as *const FormatEnum);
        let start = e.pos.load(Ordering::SeqCst).min(e.items.len());
        let n = (celt as usize).min(e.items.len() - start);
        for (k, fe) in e.items[start..start + n].iter().enumerate() {
            *out.add(k) = *fe;
        }
        e.pos.store(start + n, Ordering::SeqCst);
        if !fetched.is_null() {
            *fetched = n as u32;
        }
        if n == celt as usize { S_OK } else { S_FALSE }
    }
}

unsafe extern "system" fn enum_skip(this: *mut c_void, celt: u32) -> HRESULT {
    // SAFETY: `this` is our enumerator.
    let e = unsafe { &*(this as *const FormatEnum) };
    let start = e.pos.load(Ordering::SeqCst);
    let end = start.saturating_add(celt as usize);
    e.pos.store(end.min(e.items.len()), Ordering::SeqCst);
    if end <= e.items.len() { S_OK } else { S_FALSE }
}

unsafe extern "system" fn enum_reset(this: *mut c_void) -> HRESULT {
    // SAFETY: `this` is our enumerator.
    unsafe { (*(this as *const FormatEnum)).pos.store(0, Ordering::SeqCst) };
    S_OK
}

unsafe extern "system" fn enum_clone(this: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: pointer checked; `this` is our enumerator.
    unsafe {
        let e = &*(this as *const FormatEnum);
        *out = FormatEnum::new(e.items.clone(), e.pos.load(Ordering::SeqCst)).cast();
    }
    S_OK
}

// --- IDropSource -------------------------------------------------------------------

#[repr(C)]
struct DropSourceVtbl {
    base: UnknownVtbl,
    query_continue_drag: unsafe extern "system" fn(*mut c_void, BOOL, u32) -> HRESULT,
    give_feedback: unsafe extern "system" fn(*mut c_void, u32) -> HRESULT,
}

#[repr(C)]
struct DropSource {
    vtbl: *const DropSourceVtbl,
    refs: AtomicU32,
    return_hwnd: isize,
    returned: AtomicBool,
}

static DROP_SOURCE_VTBL: DropSourceVtbl = DropSourceVtbl {
    base: UnknownVtbl { query_interface: src_qi, add_ref: src_add_ref, release: src_release },
    query_continue_drag: src_query_continue,
    give_feedback: src_give_feedback,
};

impl DropSource {
    fn new(return_hwnd: isize) -> *mut DropSource {
        Box::into_raw(Box::new(DropSource {
            vtbl: &DROP_SOURCE_VTBL,
            refs: AtomicU32::new(1),
            return_hwnd,
            returned: AtomicBool::new(false),
        }))
    }
}

unsafe extern "system" fn src_qi(this: *mut c_void, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || riid.is_null() {
        return E_POINTER;
    }
    // SAFETY: pointers checked; `this` is our drop source.
    unsafe {
        if guid_eq(&*riid, &IID_IUNKNOWN) || guid_eq(&*riid, &IID_IDROPSOURCE) {
            src_add_ref(this);
            *out = this;
            S_OK
        } else {
            *out = ptr::null_mut();
            E_NOINTERFACE
        }
    }
}

unsafe extern "system" fn src_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live DropSource.
    unsafe { (*(this as *const DropSource)).refs.fetch_add(1, Ordering::SeqCst) + 1 }
}

unsafe extern "system" fn src_release(this: *mut c_void) -> u32 {
    // SAFETY: `this` is a live DropSource; the last reference frees the box.
    unsafe {
        let left = (*(this as *const DropSource)).refs.fetch_sub(1, Ordering::SeqCst) - 1;
        if left == 0 {
            drop(Box::from_raw(this as *mut DropSource));
        }
        left
    }
}

/// Is the pointer over the top-level window `hwnd` right now?
fn pointer_over(hwnd: isize) -> bool {
    if hwnd == 0 {
        return false;
    }
    let mut pt = POINT { x: 0, y: 0 };
    // SAFETY: plain user32 queries with a valid out pointer.
    unsafe {
        if GetCursorPos(&mut pt) == 0 {
            return false;
        }
        let under = WindowFromPoint(pt);
        !under.is_null() && GetAncestor(under, GA_ROOT) as isize == hwnd
    }
}

unsafe extern "system" fn src_query_continue(this: *mut c_void, escape: BOOL, keys: u32) -> HRESULT {
    // SAFETY: `this` is our drop source.
    let src = unsafe { &*(this as *const DropSource) };
    let decision = continue_decision(escape != 0, keys & MK_LBUTTON != 0, pointer_over(src.return_hwnd));
    match decision {
        Continue::Go => S_OK,
        Continue::Drop => DRAGDROP_S_DROP,
        Continue::Cancel => DRAGDROP_S_CANCEL,
        Continue::Return => {
            src.returned.store(true, Ordering::SeqCst);
            DRAGDROP_S_CANCEL
        }
    }
}

unsafe extern "system" fn src_give_feedback(_: *mut c_void, _effect: u32) -> HRESULT {
    DRAGDROP_S_USEDEFAULTCURSORS
}

/// `IDropSource::QueryContinueDrag` decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Continue {
    Go,
    Drop,
    Cancel,
    /// Back over our own window: cancel the OLE drag and resume the internal one.
    Return,
}

/// Esc cancels, releasing the button drops, coming back over our window returns.
pub(crate) fn continue_decision(escape: bool, button_down: bool, over_own_window: bool) -> Continue {
    if escape {
        Continue::Cancel
    } else if !button_down {
        Continue::Drop
    } else if over_own_window {
        Continue::Return
    } else {
        Continue::Go
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions() {
        assert_eq!(continue_decision(true, true, false), Continue::Cancel);
        assert_eq!(continue_decision(true, false, true), Continue::Cancel, "Esc wins");
        assert_eq!(continue_decision(false, false, true), Continue::Drop, "release drops even over us");
        assert_eq!(continue_decision(false, true, true), Continue::Return);
        assert_eq!(continue_decision(false, true, false), Continue::Go);
    }

    #[test]
    fn data_object_vtable_round_trip() {
        let cf_custom = register_format("Soyuz Test Format");
        assert!(cf_custom >= 0xC000);
        let formats = vec![(15u16, vec![1, 2, 3, 4]), (cf_custom, b"hello".to_vec())];
        let performed = register_format("Performed DropEffect");
        let (got, echoed) = read_back(formats.clone(), Some((performed, 2u32.to_le_bytes().to_vec()))).unwrap();
        assert_eq!(got.len(), 2);
        for ((cf, want), (gcf, gbytes)) in formats.iter().zip(&got) {
            assert_eq!(cf, gcf);
            // GlobalSize may round up; the payload is a prefix.
            assert_eq!(&gbytes[..want.len()], &want[..]);
        }
        assert_eq!(&echoed.unwrap()[..4], &2u32.to_le_bytes());
    }

    #[test]
    fn refuses_unknown_formats_and_counts_refs() {
        let obj = DataObject::new(vec![(13, vec![0, 0])]);
        let this = obj.cast::<c_void>();
        // SAFETY: test drives our own object through its vtable.
        unsafe {
            let vt = &*(*obj).vtbl;
            let fe = formatetc(15);
            assert_eq!((vt.query_get_data)(this, &fe), DV_E_FORMATETC);
            let mut fe2 = formatetc(13);
            fe2.tymed = 4; // TYMED_ISTREAM only
            assert_eq!((vt.query_get_data)(this, &fe2), DV_E_TYMED);
            let mut out: *mut c_void = ptr::null_mut();
            assert_eq!((vt.base.query_interface)(this, &IID_IDATAOBJECT, &mut out), S_OK);
            assert_eq!(out, this);
            assert_eq!((vt.base.query_interface)(this, &IID_IDROPSOURCE, &mut out), E_NOINTERFACE);
            assert_eq!((vt.base.release)(this), 1);
            assert_eq!((vt.base.release)(this), 0);
        }
    }

    #[test]
    fn enumerator_skip_reset_clone() {
        let en = FormatEnum::new(vec![formatetc(1), formatetc(2), formatetc(3)], 0);
        let this = en.cast::<c_void>();
        // SAFETY: test drives our own enumerator through its vtable.
        unsafe {
            let vt = &*(*en).vtbl;
            let mut buf = [FORMATETC::default(); 4];
            let mut n = 0;
            assert_eq!((vt.skip)(this, 1), S_OK);
            assert_eq!((vt.next)(this, 4, buf.as_mut_ptr(), &mut n), S_FALSE);
            assert_eq!(n, 2);
            assert_eq!((buf[0].cfFormat, buf[1].cfFormat), (2, 3));
            assert_eq!((vt.reset)(this), S_OK);
            let mut c: *mut c_void = ptr::null_mut();
            assert_eq!((vt.clone)(this, &mut c), S_OK);
            let cvt = &**(c as *mut *const EnumVtbl);
            assert_eq!((cvt.next)(c, 1, buf.as_mut_ptr(), ptr::null_mut()), S_OK);
            assert_eq!(buf[0].cfFormat, 1);
            release(c);
            release(this);
        }
    }
}
