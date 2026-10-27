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
// 次に生成する子ダイアログのタイトルバー テーマ (0 = 継承しない / 1 = Light /
// 2 = Dark)。既定では open_child_modal が親ウィンドウの現在テーマを引き継ぐ
// (ダイアログのテーマは親に揃えるのが OS の慣習)。Windows は生成フックで
// 属性に焼く (X11/macOS は slint バックエンドが生成直後にホストのカラースキーム
// を流し込むため生成後の set_theme で付け直す — child.rs 参照)。
static NEXT_CHILD_THEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
// (Windows) 子ダイアログの所有者となるメインウィンドウの HWND。
#[cfg(windows)]
static OWNER_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 次に生成する子ダイアログのタイトルバー テーマを指定する。
/// `None` で継承をやめ、OS 既定に任せる。
pub(crate) fn set_next_child_theme(theme: Option<slint::winit_030::winit::window::Theme>) {
	use slint::winit_030::winit::window::Theme;
	let v = match theme {
		Some(Theme::Dark) => 2,
		Some(Theme::Light) => 1,
		_ => 0,
	};
	NEXT_CHILD_THEME.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// 予約済みテーマを参照する (消費しない。X11/macOS では child.rs が slint
/// バックエンドのカラースキーム push に勝つまで短時間再主張する)。
#[cfg(not(windows))]
pub(crate) fn peek_next_child_theme() -> Option<slint::winit_030::winit::window::Theme> {
	use slint::winit_030::winit::window::Theme;
	match NEXT_CHILD_THEME.load(std::sync::atomic::Ordering::Relaxed) {
		1 => Some(Theme::Light),
		2 => Some(Theme::Dark),
		_ => None,
	}
}

/// フック共通 (Windows): 子ダイアログ属性へテーマを焼く。子ダイアログ以外の
/// 生成ではフラグをクリアして既定に任せる。
#[cfg(windows)]
fn apply_next_child_theme(
	child_dialog: bool,
	attrs: slint::winit_030::winit::window::WindowAttributes,
) -> slint::winit_030::winit::window::WindowAttributes {
	use slint::winit_030::winit::window::Theme;
	if !child_dialog {
		NEXT_CHILD_THEME.store(0, std::sync::atomic::Ordering::Relaxed);
		return attrs;
	}
	match NEXT_CHILD_THEME.swap(0, std::sync::atomic::Ordering::Relaxed) {
		1 => attrs.with_theme(Some(Theme::Light)),
		2 => attrs.with_theme(Some(Theme::Dark)),
		_ => attrs,
	}
}

/// 次に生成する Slint ウィンドウを「モーダル子ダイアログ」として作るよう
/// 予約する。生成の直前に呼ぶ。
///
/// オーバーレイ モードでは子ウィンドウを生成しないため何もしない。
pub fn mark_next_window_as_child_dialog() {
	if !MODAL_MODE.is_child_window() {
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
	if !MODAL_MODE.is_child_window() {
		return;
	}
	OWNER_HWND.store(hwnd, std::sync::atomic::Ordering::Relaxed);
}

/// (Windows) 子ダイアログの所有者を Slint ウィンドウから直接登録する。
///
/// アプリ側で HWND を取り回さずに済むよう、プラットフォーム固有の取得も
/// 本クレート内で行う。メインウィンドウ生成後に 1 度呼ぶ。
#[cfg(windows)]
pub fn note_owner_window(window: &slint::Window) {
	if !MODAL_MODE.is_child_window() {
		return;
	}
	let hwnd = ergon_slint_controls::platform::main_window_hwnd(window);
	if hwnd.is_null() {
		return;
	}
	OWNER_HWND.store(hwnd as isize, std::sync::atomic::Ordering::Relaxed);
}

/// winit バックエンドを明示構築し、子ダイアログ生成フックを仕込む。
/// `main` の冒頭 (最初のウィンドウ生成前) に呼ぶ。
///
/// オーバーレイ モードでは子ウィンドウを生成しないため、フックも不要。
/// 何もせず既定の (winit) バックエンドに任せる。
pub fn prepare_backend() -> Result<(), slint::PlatformError> {
	if !MODAL_MODE.is_child_window() {
		return Ok(());
	}
	// Windows: 子ダイアログを生成時点からダイアログ枠 (WS_EX_DLGMODALFRAME) +
	// 所有ウィンドウ + CenterParent の位置 + 親のテーマで作る。生成後の移動や
	// スタイル修正はちらつきの原因になる (Ubuntu 実機で WM 中央配置との二度
	// 手間がちらつきとして観測された教訓)。winit パッチ フォークの
	// with_dialog_modal_frame / (上流の) with_owner_window が WindowFlags と
	// して保持するため、winit のスタイル再計算でも外れない。メインウィンドウ
	// 自体には何もしない。
	#[cfg(windows)]
	{
		use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
		let backend = i_slint_backend_winit::Backend::builder()
			.with_window_attributes_hook(|attrs| {
				let child =
					NEXT_WINDOW_IS_CHILD_DIALOG.swap(false, std::sync::atomic::Ordering::Relaxed);
				if !child {
					return apply_next_child_theme(false, attrs);
				}
				// ダイアログ枠 (WS_EX_DLGMODALFRAME) は winit パッチ フォーク
				// 由来の API (フォーク無しのビルドは compile_error で排除済み)。
				#[cfg(feature = "winit-modal-hints")]
				let attrs = attrs.with_dialog_modal_frame(true);
				let owner = OWNER_HWND.load(std::sync::atomic::Ordering::Relaxed);
				let attrs = if owner == 0 {
					attrs
				} else {
					attrs.with_owner_window(owner as _)
				};
				let attrs = center_over_owner(attrs, owner);
				apply_next_child_theme(true, attrs)
			})
			.build()?;
		slint::platform::set_platform(Box::new(backend))
			.map_err(|e| slint::PlatformError::Other(e.to_string()))?;
	}
	// X11: 子ダイアログを生成時点で _NET_WM_WINDOW_TYPE_DIALOG にする。
	// ダイアログ型はウィンドウ マネージャが「タイトルバーのメニュー無し・
	// タスクバーに出さない」等の扱いをする。transient / modal は生成後に
	// child モジュールが付ける。位置は座標指定せず WM の配置 (transient なら
	// 親基準) に任せる。テーマは生成後に child.rs が set_theme する (slint
	// バックエンドが生成直後にホストのカラースキームを流し込むため、生成時
	// 属性では上書きされてしまう)。
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		use slint::winit_030::winit::platform::x11::{WindowAttributesExtX11, WindowType};
		let backend = i_slint_backend_winit::Backend::builder()
			.with_window_attributes_hook(|attrs| {
				let child =
					NEXT_WINDOW_IS_CHILD_DIALOG.swap(false, std::sync::atomic::Ordering::Relaxed);
				if !child {
					NEXT_CHILD_THEME.store(0, std::sync::atomic::Ordering::Relaxed);
					attrs
				} else {
					attrs.with_x11_window_type(vec![WindowType::Dialog])
				}
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

/// (Windows) 子ダイアログを所有ウィンドウ (親) の中央に置く。Windows の
/// `StartPosition.CenterParent` 相当。生成後の SetWindowPos 移動は一度
/// 別位置で描画された後に動くためちらつくが、生成属性 (`with_position`) なら
/// 最初の描画から正しい位置になる。
#[cfg(windows)]
fn center_over_owner(
	mut attrs: slint::winit_030::winit::window::WindowAttributes,
	owner: isize,
) -> slint::winit_030::winit::window::WindowAttributes {
	use slint::winit_030::winit::dpi::{PhysicalPosition, Position};
	use windows_sys::Win32::Foundation::RECT;
	use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
	use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
	let Some(size) = attrs.inner_size else {
		return attrs;
	};
	if owner == 0 {
		return attrs;
	}
	let mut pr = RECT::default();
	unsafe {
		if GetWindowRect(owner as _, &mut pr) == 0 {
			return attrs;
		}
	}
	// 親の DPI を基準にスケール変換する (子は親と同じモニタに出る想定)。
	let scale = unsafe { GetDpiForWindow(owner as _) } as f64 / 96.0;
	let cs: slint::winit_030::winit::dpi::PhysicalSize<u32> = size.to_physical(scale);
	let x = pr.left + ((pr.right - pr.left) - cs.width as i32) / 2;
	let y = pr.top + ((pr.bottom - pr.top) - cs.height as i32) / 2;
	attrs.with_position(Position::Physical(PhysicalPosition::new(x, y)))
}
