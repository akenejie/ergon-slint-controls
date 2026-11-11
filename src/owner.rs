//! Windows / Linux: ネイティブ ダイアログとモーダル子ダイアログが共有する
//! 「所有者ウィンドウ」の管理。
//!
//! 所有者は**スタック**で管理する (入れ子のモーダル / 複数ウィンドウ対応):
//!
//! - **底 (基本)**: 最初のウィンドウ (platform::setup_window が capture す
//!   る)。どのウィンドウが親になるかをアプリが指定しなかった場合のフォール
//!   バック。
//! - **積まれた分**: `dialogs::dialog_push(parent)` の各区間
//!   (`dialog_pop` で外れる)。アプリは「どのウィンドウの子としてダイアログ
//!   を開くか」を区間ごとに指定する (入れ子のモーダルでは、そのとき開いてい
//!   るダイアログのウィンドウを渡す)。
//!
//! ダイアログ (TaskDialog / MessageBox / GTK Dialog) はスタックの先頭を親に
//! する。別系統の「子ダイアログ (ウィンドウ方式) の生成時 owner」は
//! [`set_marked_owner`] で保持する (modal_dialog の mark 専用)。

#[cfg(windows)]
use std::sync::atomic::{AtomicPtr, Ordering};

// --- Windows: HWND ---

#[cfg(windows)]
static BASE_HWND: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// HWND の生ポインタ (スレッド越えを許すラッパ。HWND はプロセス全体で有効)。
#[cfg(windows)]
#[derive(Clone, Copy)]
struct Hwnd(*mut core::ffi::c_void);
#[cfg(windows)]
unsafe impl Send for Hwnd {}

#[cfg(windows)]
static OWNER_STACK: std::sync::Mutex<Vec<Hwnd>> = std::sync::Mutex::new(Vec::new());

// --- Linux: X ウィンドウ ID ---

#[cfg(all(unix, not(target_os = "macos")))]
static BASE_XID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

#[cfg(all(unix, not(target_os = "macos")))]
static OWNER_STACK_X: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

// --- 子ダイアログ (ウィンドウ方式) の生成時 owner (modal_dialog の mark 専用) ---

#[cfg(windows)]
static MARKED_HWND: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// 基本 (底) の所有者を設定する。既に設定済みなら何もしない。
#[cfg(windows)]
pub(crate) fn set_base_owner_hwnd(hwnd: *mut core::ffi::c_void) {
	if hwnd.is_null() {
		return;
	}
	let _ = BASE_HWND.compare_exchange(
		std::ptr::null_mut(),
		hwnd,
		Ordering::Relaxed,
		Ordering::Relaxed,
	);
}

/// 所有者スタックへ積む。parent からプラットフォーム固有の ID を解決できな
/// ければ何もしない (フォールバックが使われる)。
pub(crate) fn push_owner(parent: &slint::Window) {
	#[cfg(windows)]
	{
		let hwnd = crate::platform::window_hwnd(parent);
		if hwnd.is_null() {
			return;
		}
		if let Ok(mut stack) = OWNER_STACK.lock() {
			stack.push(Hwnd(hwnd));
		}
	}
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		// XID は公式の raw-window-handle 経由で取得する (platform.rs の
		// winit_xlib_window_id — フォークなしの winit でも動く)。
		use slint::winit_030::WinitWindowAccessor;
		let xid = parent
			.with_winit_window(crate::platform::winit_xlib_window_id)
			.flatten();
		let Some(xid) = xid else { return };
		if let Ok(mut stack) = OWNER_STACK_X.lock() {
			stack.push(xid as u32);
		}
	}
	let _ = parent;
}

/// 所有者スタックから外す (底は外れない)。
pub(crate) fn pop_owner() {
	#[cfg(windows)]
	if let Ok(mut stack) = OWNER_STACK.lock() {
		stack.pop();
	}
	#[cfg(all(unix, not(target_os = "macos")))]
	if let Ok(mut stack) = OWNER_STACK_X.lock() {
		stack.pop();
	}
}

/// 現在の所有者 (Windows: HWND)。スタックの先頭、無ければ底。未登録は null。
#[cfg(windows)]
pub(crate) fn current_owner_hwnd() -> *mut core::ffi::c_void {
	if let Ok(stack) = OWNER_STACK.lock() {
		if let Some(hwnd) = stack.last() {
			return hwnd.0;
		}
	}
	BASE_HWND.load(Ordering::Relaxed)
}

/// 現在の所有者 (Linux: X ウィンドウ ID)。スタックの先頭、無ければ底。
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn current_owner_xid() -> Option<u32> {
	if let Ok(stack) = OWNER_STACK_X.lock() {
		if let Some(xid) = stack.last() {
			return Some(*xid);
		}
	}
	let base = BASE_XID.load(std::sync::atomic::Ordering::Relaxed);
	(base != 0).then_some(base)
}

// --- 子ダイアログ (ウィンドウ方式) の生成時 owner ---

/// 子ダイアログ (ウィンドウ方式) の生成時 owner を登録する
/// (modal_dialog::note_owner_window / note_owner_hwnd 専用)。
#[cfg(windows)]
pub(crate) fn set_marked_owner_hwnd(hwnd: *mut core::ffi::c_void) {
	if !hwnd.is_null() {
		MARKED_HWND.store(hwnd, Ordering::Relaxed);
	}
}

/// 子ダイアログ (ウィンドウ方式) の生成時 owner。未登録は null。
#[cfg(windows)]
pub(crate) fn marked_owner_hwnd() -> *mut core::ffi::c_void {
	MARKED_HWND.load(Ordering::Relaxed)
}

#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
pub(crate) fn set_base_owner(parent: &slint::Window) {
	let _ = parent;
}