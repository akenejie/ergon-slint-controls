//! Windows: ネイティブ ダイアログとモーダル子ダイアログが共有する「所有者
//! ウィンドウ」レジストリ。
//!
//! メイン ウィンドウの HWND を 1 か所に保持し、MessageBox / TaskDialog の
//! 親や、子ダイアログ生成時の owner 属性に使う。ウィンドウが再生成されても
//! 最初の HWND を維持する (OS のモーダル オーナーは最初のもので足りる)。

#[cfg(windows)]
use std::sync::atomic::{AtomicPtr, Ordering};

#[cfg(windows)]
static OWNER_HWND: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

/// 最初の非 null 値だけを記録する (既に記録済みなら何もしない)。
#[cfg(windows)]
pub(crate) fn set_owner_hwnd_once(hwnd: *mut core::ffi::c_void) {
	if hwnd.is_null() {
		return;
	}
	let _ = OWNER_HWND.compare_exchange(
		std::ptr::null_mut(),
		hwnd,
		Ordering::Relaxed,
		Ordering::Relaxed,
	);
}

/// 明示的に所有者を更新する (子ダイアログの note_owner_window 用)。
#[cfg(windows)]
pub(crate) fn set_owner_hwnd(hwnd: *mut core::ffi::c_void) {
	if !hwnd.is_null() {
		OWNER_HWND.store(hwnd, Ordering::Relaxed);
	}
}

/// 登録済みの所有者 HWND (未登録は null)。
#[cfg(windows)]
pub(crate) fn owner_hwnd() -> *mut core::ffi::c_void {
	OWNER_HWND.load(Ordering::Relaxed)
}
