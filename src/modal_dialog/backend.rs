//! モーダル子ダイアログ用のウィンドウ生成属性 (OS 差分)。
//!
//! 「次に生成する Slint ウィンドウをモーダル子ダイアログとして作る」ための
//! OS 固有属性を、winit バックエンドの `window_attributes_hook` に仕込む。
//! Slint は生成時点でタイトルを設定しないため、後から見た目を変えるより
//! 生成時に属性を決める方が確実 (タイトルバーに exe アイコンが一瞬補完される
//! 之类的な問題を避けられる)。

// 次に生成するウィンドウがモーダル子ダイアログであることをフックへ伝える
// フラグ。mark_next_window_as_child_dialog() の直前に立て、フックが swap で
// 消費する。
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
static NEXT_WINDOW_IS_CHILD_DIALOG: std::sync::atomic::AtomicBool =
	std::sync::atomic::AtomicBool::new(false);
// 次に生成する子ダイアログの transient 親 (X11 ウィンドウ ID、0 = 指定なし)。
// mark_next_child_dialog_of() が立て、生成フックが swap で消費する。生成時
// (XMapWindow と同じ要求ストリーム) に WM_TRANSIENT_FOR を焼き込むため、
// slint 側がどのタイミングでマップしても WM は transient を解決できる。
#[cfg(all(unix, not(target_os = "macos")))]
static NEXT_CHILD_TRANSIENT_PARENT: std::sync::atomic::AtomicU32 =
	std::sync::atomic::AtomicU32::new(0);
// 次に生成する子ダイアログのタイトルバー テーマ (0 = 継承しない / 1 = Light /
// 2 = Dark)。既定では open_child_modal が親ウィンドウの現在テーマを引き継ぐ
// (ダイアログのテーマは親に揃えるのが OS の慣習)。Windows は生成フックで
// 属性に焼く (X11/macOS は slint バックエンドが生成直後にホストのカラースキーム
// を流し込むため生成後の set_theme で付け直す — child.rs 参照)。
static NEXT_CHILD_THEME: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
// (Windows) 子ダイアログの所有者となるメインウィンドウの HWND は
// crate::owner のレジストリをネイティブ ダイアログと共有する。

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
/// 子ウィンドウ方式でのみ呼ぶこと (オーバーレイ方式のアプリは呼ばない)。
pub fn mark_next_window_as_child_dialog() {
	#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
	NEXT_WINDOW_IS_CHILD_DIALOG.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// 次に生成する Slint ウィンドウを「`parent` のモーダル子ダイアログ」として
/// 作るよう予約する。子コンポーネント生成の直前に呼ぶ。
///
/// [`mark_next_window_as_child_dialog`] と同じ役割に加え、X11 では親ウィンド
/// ウ ID を生成フックへ渡し、**ウィンドウ生成時に WM_TRANSIENT_FOR を書き込
/// む**。winit の post-creation API (set_transient_for) は生成後の適用になる
/// ため、slint が show() を遅延マップする場合などに transient 無しでマップ
/// されるレースがあり、Mutter は transient 無しで管理した位置 (親基準でな
/// い) を確定させてしまう。生成時に焼き込むことでこれを防ぐ。
#[cfg_attr(windows, allow(unused_variables))]
pub fn mark_next_child_dialog_of(parent: &slint::Window) {
	mark_next_window_as_child_dialog();
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		// XID は公式の raw-window-handle 経由で取得する (platform.rs の
		// winit_xlib_window_id — フォークなしの winit でも動く)。
		use slint::winit_030::WinitWindowAccessor;
		let xid = parent
			.with_winit_window(crate::platform::winit_xlib_window_id)
			.flatten()
			.unwrap_or(0);
		NEXT_CHILD_TRANSIENT_PARENT.store(xid, std::sync::atomic::Ordering::Relaxed);
	}
}

/// (Windows) 子ダイアログの所有者となるメインウィンドウの HWND を登録する。
/// メインウィンドウ生成後に 1 度呼ぶ。未登録のまま子ダイアログを生成すると
/// 所有者なしのウィンドウになる (タスクバーに別枠で出るなど)。
#[cfg(windows)]
pub fn note_owner_hwnd(hwnd: isize) {
	crate::owner::set_marked_owner_hwnd(hwnd as *mut core::ffi::c_void);
}

/// (Windows) 子ダイアログの所有者を Slint ウィンドウから直接登録する。
///
/// アプリ側で HWND を取り回さずに済むよう、プラットフォーム固有の取得も
/// 本クレート内で行う。メインウィンドウ生成後に 1 度呼ぶ。
#[cfg(windows)]
pub fn note_owner_window(window: &slint::Window) {
	let hwnd = crate::platform::window_hwnd(window);
	if hwnd.is_null() {
		return;
	}
	crate::owner::set_marked_owner_hwnd(hwnd);
}

/// winit バックエンドを明示構築し、子ダイアログ生成フックを仕込む。
/// `main` の冒頭 (最初のウィンドウ生成前) に呼ぶ。
///
/// 子ウィンドウ方式のときに呼ぶ。オーバーレイ方式では既定の (winit)
/// バックエンドでよい (呼んでも、フックは mark_* で予約が立つまで働かない
/// ため害はない)。
pub fn prepare_backend() -> Result<(), slint::PlatformError> {
	// Windows: 子ダイアログを生成時点からダイアログ枠 (WS_EX_DLGMODALFRAME) +
	// 所有ウィンドウ付きで作り、タイトルバーのテーマも親に揃える。
	//
	// 位置: Windows は OS がダイアログの位置を決めないためライブラリの責任だが、
	// 生成時にはまだサイズが確定しない (slint は表示直前にリサイズする)。そこで
	// winit パッチ フォークの with_dialog_center_over_owner を指示し、フォークが
	// 表示直前 (ShowWindow の直前、実矩形が出揃った時点) に所有者の中央へ配置
	// する。最初のフレームから正位置で、表示後の移動によるちらつきはない。
	// 枠もフォークの with_dialog_modal_frame / (上流の) with_owner_window が
	// WindowFlags として保持するため、winit のスタイル再計算でも外れない。
	// メインウィンドウ自体には何もしない。
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
				// ダイアログ枠と位置 (表示直前の所有者中央配置) は winit パッチ
				// フォーク由来の API (fork は modal-dialog feature が要求する)。
				let attrs = attrs.with_dialog_modal_frame(true);
				let owner = crate::owner::marked_owner_hwnd();
				let attrs = if owner.is_null() {
					attrs
				} else {
					attrs.with_owner_window(owner as isize)
				};
				let attrs = attrs.with_dialog_center_over_owner(true);
				apply_next_child_theme(true, attrs)
			})
			.build()?;
		slint::platform::set_platform(Box::new(backend))
			.map_err(|e| slint::PlatformError::Other(e.to_string()))?;
	}
	// X11: 子ダイアログを生成時点で _NET_WM_WINDOW_TYPE_DIALOG と
	// WM_TRANSIENT_FOR (transient 親) にする。ダイアログ型はウィンドウ マネー
	// ジャが「タイトルバーのメニュー無し・タスクバーに出さない」等の扱いをす
	// る。transient を生成時に焼き込むのは、Mutter が transient を解決するの
	// は管理開始時 (最初のマップ) 一度きりで、解決できないと親基準の配置を行
	// わないため。slint の show() は遅延マップであり、post-creation の
	// set_transient_for では間に合わないレースが起きる。modal は生成後に
	// child モジュールが付ける。位置は座標指定せず WM の配置 (transient なら
	// 親基準) に任せる。テーマは生成後に child.rs が set_theme する (slint
	// バックエンドが生成直後にホストのカラースキームを流し込むため、生成時
	// 属性では上書きされてしまう)。
	// また X11 では非表示 (with_visible(false)) で生成する: transient / modal
	// ヒントを **マップ前に** 付けるため。マップ後にヒントを付けると WM は
	// transient 無しの配置 (スマート配置 / 前回からのカスケード) を先に決め
	// てしまい、Ubuntu (Mutter) で 2 回目以降の表示位置が親基準からずれる。
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		use slint::winit_030::winit::platform::x11::{WindowAttributesExtX11, WindowType};
		use slint::winit_030::winit::window::WindowButtons;
		let backend = i_slint_backend_winit::Backend::builder()
			.with_window_attributes_hook(|attrs| {
				let child =
					NEXT_WINDOW_IS_CHILD_DIALOG.swap(false, std::sync::atomic::Ordering::Relaxed);
				if !child {
					NEXT_CHILD_THEME.store(0, std::sync::atomic::Ordering::Relaxed);
					NEXT_CHILD_TRANSIENT_PARENT.store(0, std::sync::atomic::Ordering::Relaxed);
					attrs
				} else {
					let parent = NEXT_CHILD_TRANSIENT_PARENT.swap(0, std::sync::atomic::Ordering::Relaxed);
					attrs
						.with_x11_window_type(vec![WindowType::Dialog])
						.with_x11_transient_for(if parent != 0 { Some(parent) } else { None })
						// モーダル状態も生成時に焼き込む (ClientMessage はマップ
						// 前だと WM に無視されるため、管理開始時に読ませる)。
						.with_x11_modal(true)
						// 固定サイズ + 閉じるボタンだけも生成時に焼き込む。生成
						// 後 (マップ後) の MWM ヒント変更は WM に再装飾させ、表
						// 示位置がわずかにズレるちらつきの原因になる。
						.with_resizable(false)
						.with_enabled_buttons(WindowButtons::CLOSE)
						.with_visible(false)
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
