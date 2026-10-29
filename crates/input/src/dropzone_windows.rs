use super::{Drop, Zone};
use anyhow::Result;
use one_mk_proto::{Pos, drag::{Change, Drag}};
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use std::os::windows::ffi::OsStringExt;
use tokio::sync::{mpsc, watch};
use windows::core::{implement, w, Ref};
use windows::Win32::{Foundation::*, System::{Com::*, Ole::*, SystemServices::MODIFIERKEYS_FLAGS, Threading::GetCurrentThreadId},
    UI::{Shell::{DragQueryFileW, HDROP}, WindowsAndMessaging::*}};

thread_local! { static ACTIVE: RefCell<Option<Rc<RefCell<State>>>> = const { RefCell::new(None) }; }

struct State {
    hwnd: HWND,
    zones: Vec<Zone>,
    shown: Option<usize>,
    drag: Drag,
    held: bool,
    accepts: bool,
    dropped: mpsc::Sender<Drop>,
}

impl State {
    fn hide(&mut self) {
        self.shown = None;
        self.accepts = false;
        unsafe { let _ = ShowWindow(self.hwnd, SW_HIDE); KillTimer(Some(self.hwnd), 1).ok(); }
    }

    fn pointer(&mut self, point: POINT) {
        let zone = self.zones.iter().position(|zone| zone.rect.contains(Pos::new(point.x.into(), point.y.into())));
        match self.drag.pointer(self.held, zone) {
            Change::Show(index) => {
                self.hide();
                let rect = self.zones[index].rect;
                unsafe { let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), rect.x, rect.y, rect.w, rect.h, SWP_NOACTIVATE | SWP_SHOWWINDOW); }
                self.shown = Some(index);
                if unsafe { SetTimer(Some(self.hwnd), 1, 30_000, None) } == 0 { self.cancel(); }
            }
            Change::Hide => self.hide(),
            Change::None => {}
        }
    }

    fn cancel(&mut self) { self.drag.cancel(); self.hide(); }
}

fn with_state(action: impl FnOnce(&mut State)) {
    ACTIVE.with(|active| {
        if let Some(state) = active.borrow().as_ref() && let Ok(mut state) = state.try_borrow_mut() { action(&mut state); }
    });
}

unsafe extern "system" fn mouse(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = unsafe { &*(data.0 as *const MSLLHOOKSTRUCT) };
        if event.flags & LLMHF_INJECTED == 0 {
            with_state(|state| match message.0 as u32 {
                WM_LBUTTONDOWN => {
                    if state.drag.pointer(false, None) == Change::Hide { state.hide(); }
                    state.held = true;
                    state.pointer(event.pt);
                }
                WM_MOUSEMOVE if state.held => state.pointer(event.pt),
                WM_LBUTTONUP => {
                    state.held = false;
                    if state.shown.is_some() { unsafe { SetTimer(Some(state.hwnd), 1, 300, None); } }
                    else { state.drag.pointer(false, None); }
                }
                _ => {}
            });
        }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

unsafe extern "system" fn keyboard(code: i32, message: WPARAM, data: LPARAM) -> LRESULT {
    if code >= 0 && message.0 as u32 == WM_KEYDOWN {
        let event = unsafe { &*(data.0 as *const KBDLLHOOKSTRUCT) };
        if event.vkCode == 0x1b { with_state(State::cancel); }
    }
    unsafe { CallNextHookEx(None, code, message, data) }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, word: WPARAM, data: LPARAM) -> LRESULT {
    if message == WM_TIMER { with_state(State::cancel); return LRESULT(0); }
    unsafe { DefWindowProcW(hwnd, message, word, data) }
}

fn format() -> FORMATETC {
    FORMATETC { cfFormat: 15, dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32, ..Default::default() }
}

#[implement(IDropTarget)]
struct OleTarget { state: Rc<RefCell<State>> }

impl IDropTarget_Impl for OleTarget_Impl {
    fn DragEnter(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let accepts = data.as_ref().is_some_and(|data| unsafe { data.QueryGetData(&format()).is_ok() });
        self.state.borrow_mut().accepts = accepts;
        self.DragOver(MODIFIERKEYS_FLAGS(0), &POINTL::default(), effect)
    }

    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let state = self.state.borrow();
        unsafe { *effect = if state.accepts && state.shown.is_some() && (*effect & DROPEFFECT_COPY) != DROPEFFECT_NONE { DROPEFFECT_COPY } else { DROPEFFECT_NONE }; }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> { self.state.borrow_mut().cancel(); Ok(()) }

    fn Drop(&self, data: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let copy = unsafe { (*effect & DROPEFFECT_COPY) != DROPEFFECT_NONE };
        unsafe { *effect = DROPEFFECT_NONE; }
        let paths = if copy { data.as_ref().and_then(|data| paths(data).ok()) } else { None };
        let mut state = self.state.borrow_mut();
        if let (Some(index), Some(paths)) = (state.shown, paths) && state.drag.drop_on(index).is_some() {
            let peer = state.zones[index].peer.clone();
            if state.dropped.try_send(Drop { peer, paths }).is_ok() { unsafe { *effect = DROPEFFECT_COPY; } }
        }
        state.cancel();
        Ok(())
    }
}

fn paths(data: &IDataObject) -> windows::core::Result<Vec<PathBuf>> {
    let mut medium = unsafe { data.GetData(&format())? };
    let result = (|| {
        if medium.tymed != TYMED_HGLOBAL.0 as u32 { return Err(windows::core::Error::from_hresult(E_INVALIDARG)); }
        let handle = HDROP(unsafe { medium.u.hGlobal.0 });
        let count = unsafe { DragQueryFileW(handle, u32::MAX, None) };
        if count == 0 || count > 10_000 { return Err(windows::core::Error::from_hresult(E_INVALIDARG)); }
        let mut paths = Vec::with_capacity(count as usize);
        let mut total = 0;
        for index in 0..count {
            let length = unsafe { DragQueryFileW(handle, index, None) } as usize;
            total += length;
            if length == 0 || total > super::MAX_URI_BYTES / 2 { return Err(windows::core::Error::from_hresult(E_INVALIDARG)); }
            let mut path = vec![0; length + 1];
            if unsafe { DragQueryFileW(handle, index, Some(&mut path)) } as usize != length { return Err(windows::core::Error::from_hresult(E_INVALIDARG)); }
            paths.push(PathBuf::from(std::ffi::OsString::from_wide(&path[..length])));
        }
        Ok(paths)
    })();
    unsafe { ReleaseStgMedium(&mut medium); }
    result
}

struct Resources { hwnd: HWND, mouse: HHOOK, keyboard: HHOOK }
impl std::ops::Drop for Resources {
    fn drop(&mut self) {
        unsafe {
            let _ = UnhookWindowsHookEx(self.mouse);
            let _ = UnhookWindowsHookEx(self.keyboard);
            let _ = RevokeDragDrop(self.hwnd);
            let _ = DestroyWindow(self.hwnd);
            OleUninitialize();
        }
    }
}

pub(super) fn start(mut zones: watch::Receiver<Vec<Zone>>, dropped: mpsc::Sender<Drop>) -> Result<()> {
    let mut changes = zones.clone();
    let (ready, initialized) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new().name("one-mk-drop".into()).spawn(move || {
        let mut run = || -> windows::core::Result<()> {
            unsafe {
                OleInitialize(None)?;
                let mut resources = Resources { hwnd: HWND::default(), mouse: HHOOK::default(), keyboard: HHOOK::default() };
                let module = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?;
                let instance = HINSTANCE(module.0);
                let class = WNDCLASSW { lpfnWndProc: Some(procedure), lpszClassName: w!("OneMkFileDrop"),
                    hInstance: instance, hbrBackground: windows::Win32::Graphics::Gdi::GetSysColorBrush(windows::Win32::Graphics::Gdi::COLOR_HIGHLIGHT), ..Default::default() };
                RegisterClassW(&class);
                let hwnd = CreateWindowExW(WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW, class.lpszClassName,
                    w!("Drop files to paired computer"), WS_POPUP, 0, 0, 1, 1, None, None, Some(instance), None)?;
                resources.hwnd = hwnd;
                let state = Rc::new(RefCell::new(State { hwnd, zones: zones.borrow_and_update().clone(), shown: None,
                    drag: Drag::default(), held: false, accepts: false, dropped: dropped.clone() }));
                let target: IDropTarget = OleTarget { state: state.clone() }.into();
                RegisterDragDrop(hwnd, &target)?;
                resources.mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), Some(instance), 0)?;
                resources.keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), Some(instance), 0)?;
                ACTIVE.with(|active| *active.borrow_mut() = Some(state.clone()));
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                let _ = ready.send(Ok(GetCurrentThreadId()));
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    if message.message == WM_APP {
                        let mut state = state.borrow_mut();
                        state.cancel();
                        state.zones = zones.borrow_and_update().clone();
                    } else { let _ = TranslateMessage(&message); DispatchMessageW(&message); }
                }
                ACTIVE.with(|active| *active.borrow_mut() = None);
                std::mem::drop(resources);
                Ok(())
            }
        };
        if let Err(error) = run() { let _ = ready.send(Err(error.to_string())); }
    })?;
    let thread = initialized.recv().map_err(|error| anyhow::anyhow!(error))?.map_err(anyhow::Error::msg)?;
    tokio::spawn(async move {
        while changes.changed().await.is_ok() {
            unsafe { if PostThreadMessageW(thread, WM_APP, WPARAM(0), LPARAM(0)).is_err() { return; } }
        }
        unsafe { let _ = PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)); }
    });
    Ok(())
}
