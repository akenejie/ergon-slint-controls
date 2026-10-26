//! モーダル子ダイアログ用のウィンドウ生成属性 (OS 差分)。
//!
//! 「次に生成する Slint ウィンドウをモーダル子ダイアログとして作る」ための
//! OS 固有属性を、winit バックエンドの `window_attributes_hook` に仕込む。
//! Slint は生成時点でタイトルを設定しないため、後から見た目を変えるより
//! 生成時に属性を決める方が確実 (タイトルバーに exe アイコンが一瞬補完される
//! 之类的な問題を避けられる)。

use crate::MODAL_MODE;

// 次に生成するウィンドウがモーダル子ダイアログであることをフックへ伝える
// フラグ。mark_next_window_as_child_dialog() の直前に立て、フックが swap で
// 消費する。
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
static NEXT_WINDOW_IS_CHILD_DIALOG: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);
// (Windows) 子ダイアログの所有者となるメインウィンドウの HWND。
#[cfg(windows)]
static OWNER_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 次に生成する Slint ウィンドウを「モーダル子ダイアログ」として作るよう
/// 予約する。生成の直前に呼ぶ。
///
/// オーバーレイ モードでは子ウィンドウを生成しないため何もしない。
pub fn mark_next_window_as_child_dialog() {
	if MODAL_MODE.is_child_window() {
		return;
	}
	#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
	NEXT_WINDOW_IS_CHILD_DIALOG.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// (Windows) 子ダイアログの所有者となるメインウィンドウの HWND を登録する。
/// メインウィンドウ生成後に 1 度呼ぶ。未登録のまま子ダイアログを生成すると
/// 所有者なしのウィンドウになる (タスクバーに別枠で出るなど)。
#[cfg(windows)]
pub fn note_owner_hwnd(hwnd: isize) {
	if MODAL_MODE.is_child_window() {
		return;
	}
	OWNER_HWND.store(hwnd, std::sync::atomic::Ordering::Relaxed);
}

/// winit バックエンドを明示構築し、子ダイアログ生成フックを仕込む。
/// `main` の冒頭 (最初のウィンドウ生成前) に呼ぶ。
///
/// オーバーレイ モードでは子ウィンドウを生成しないため、フックも不要。
/// 何もせず既定の (winit) バックエンドに任せる。
pub fn prepare_backend() -> Result<(), slint::PlatformError> {
	if MODAL_MODE.is_child_window() {
		return Ok(());
	}
	// Windows: 子ダイアログを生成時点からダイアログ枠 (WS_EX_DLGMODALFRAME) +
	// 所有ウィンドウ付きで作る。patches/winit の with_dialog_modal_frame /
	// with_owner_window が WindowFlags として保持するため、winit のスタイル
	// 再計算でも外れない。メインウィンドウ自体には何もしない。
	#[cfg(windows)]
	{
		use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
		let backend = i_slint_backend_winit::Backend::builder()
			.with_window_attributes_hook(|attrs| {
				if !NEXT_WINDOW_IS_CHILD_DIALOG.swap(false, std::sync::atomic::Ordering::Relaxed)
				{
					return attrs;
				}
				// ダイアログ枠 (WS_EX_DLGMODALFRAME) は vendored winit パッチ
				// 由来の API なので、そのパッチを使うビルドでのみ付けられる。
				#[cfg(feature = "winit-modal-hints")]
				let attrs = attrs.with_dialog_modal_frame(true);
				let owner = OWNER_HWND.load(std::sync::atomic::Ordering::Relaxed);
				if owner == 0 {
					attrs
				} else {
					attrs.with_owner_window(owner as _)
				}
			})
			.build()?;
		slint::platform::set_platform(Box::new(backend))
			.map_err(|e| slint::PlatformError::Other(e.to_string()))?;
	}
	// X11: 子ダイアログを生成時点で _NET_WM_WINDOW_TYPE_DIALOG にする。
	// ダイアログ型はウィンドウ マネージャが「タイトルバーのメニュー無し・
	// タスクバーに出さない」等の扱いをする。transient / modal は生成後に
	// child モジュールが付ける。
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		use slint::winit_030::winit::platform::x11::{WindowAttributesExtX11, WindowType};
		let backend = i_slint_backend_winit::Backend::builder()
			.with_window_attributes_hook(|attrs| {
				if !NEXT_WINDOW_IS_CHILD_DIALOG.swap(false, std::sync::atomic::Ordering::Relaxed)
				{
					return attrs;
				}
				attrs.with_x11_window_type(vec![WindowType::Dialog])
			})
			.build()?;
		slint::platform::set_platform(Box::new(backend))
			.map_err(|e| slint::PlatformError::Other(e.to_string()))?;
	}
	// その他のプラットフォームでは既定のバックエンドに任せる。
	#[cfg(not(any(windows, all(unix, not(target_os = "macos")))))]
	{
		let _ = 0;
	}
	Ok(())
}
