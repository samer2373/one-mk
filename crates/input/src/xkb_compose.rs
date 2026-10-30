use super::{LayoutResolver, LayoutState, lib};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::OnceLock;

library! { ComposeLib, load_compose, {
    table_new = c"xkb_compose_table_new_from_locale": fn(*mut c_void, *const c_char, c_int) -> *mut c_void;
    table_unref = c"xkb_compose_table_unref": fn(*mut c_void);
    state_new = c"xkb_compose_state_new": fn(*mut c_void, c_int) -> *mut c_void;
    state_unref = c"xkb_compose_state_unref": fn(*mut c_void);
    state_feed = c"xkb_compose_state_feed": fn(*mut c_void, u32) -> c_int;
    state_status = c"xkb_compose_state_get_status": fn(*mut c_void) -> c_int;
    state_utf8 = c"xkb_compose_state_get_utf8": fn(*mut c_void, *mut c_char, usize) -> c_int;
    state_reset = c"xkb_compose_state_reset": fn(*mut c_void);
}}

/// Composition is visit-local, not a complete routing policy. A caller must
/// buffer Pending keys and keep their ups paired when replaying physical input.
#[derive(Debug, PartialEq, Eq)]
pub enum Composition {
    Text(String),
    Pending,
    Cancelled,
    Physical,
}

/// Optional locale-specific compose state, with the same text validation as
/// the wire codec. This does not read or write the clipboard or inject input.
pub struct LayoutComposer {
    library: &'static ComposeLib,
    state: *mut c_void,
}

impl LayoutComposer {
    /// Absence of compose symbols or tables must fall back to physical keys for
    /// the whole sequence. The locale is supplied explicitly by the caller.
    pub fn new(locale: &CStr) -> Option<Self> {
        static COMPOSE: OnceLock<Option<ComposeLib>> = OnceLock::new();
        let library = COMPOSE.get_or_init(|| unsafe { load_compose() }).as_ref()?;
        let core = lib()?;
        unsafe {
            let context = (core.context_new)(0);
            if context.is_null() { return None; }
            let table = (library.table_new)(context, locale.as_ptr(), 0);
            (core.context_unref)(context);
            if table.is_null() { return None; }
            let state = (library.state_new)(table, 0);
            (library.table_unref)(table);
            if state.is_null() { return None; }
            Some(Self { library, state })
        }
    }

    /// Reset at visit end, layout change and after abandoning a sequence.
    pub fn reset(&mut self) {
        unsafe { (self.library.state_reset)(self.state) };
    }

    pub fn feed(&mut self, resolver: &LayoutResolver, code: u32, masks: LayoutState) -> Composition {
        if !one_mk_proto::typing::valid_key_code(code) {
            self.reset();
            return Composition::Physical;
        }
        let Some(query) = resolver.query(masks) else {
            self.reset();
            return Composition::Physical;
        };
        let symbol = unsafe { (query.lib.state_key_get_one_sym)(query.state, code + super::EVDEV_OFFSET) };
        unsafe { (self.library.state_feed)(self.state, symbol) };
        match unsafe { (self.library.state_status)(self.state) } {
            0 => LayoutResolver::text(&query, code).map_or(Composition::Physical, Composition::Text),
            1 => Composition::Pending,
            2 => {
                let mut bytes = [0u8; one_mk_proto::typing::MAX_TEXT_BYTES + 1];
                let length = unsafe { (self.library.state_utf8)(self.state, bytes.as_mut_ptr().cast(), bytes.len()) };
                self.reset();
                usize::try_from(length).ok().filter(|length| *length < bytes.len())
                    .and_then(|length| std::str::from_utf8(&bytes[..length]).ok())
                    .filter(|text| one_mk_proto::typing::valid_text(text))
                    .map_or(Composition::Physical, |text| Composition::Text(text.to_owned()))
            }
            _ => {
                self.reset();
                Composition::Cancelled
            }
        }
    }
}

impl Drop for LayoutComposer {
    fn drop(&mut self) {
        unsafe { (self.library.state_unref)(self.state) };
    }
}
