//! ネイティブプラットフォーム操作の汎用ヘルパ:
//! ウインドウハンドル取得、ウインドウアイコンの設定、モディファイアキーの照会。
//! アイコンは実行ファイルに埋め込まれたリソース (アイコンリソース ID は
//! 呼び出し側が渡す) を使うため、このクレートは実アイコンを持たない。

#[cfg(windows)]
pub fn encode_wide(s: &str) -> Vec<u16> {
	s.encode_utf16().chain(Some(0)).collect()
}

/// ウィンドウ装飾 (タイトルバー) のテーマ。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitlebarTheme {
	Light,
	Dark,
	/// システム既定に従う (winit へは `None` を渡す)。
	System,
}

/// ウィンドウのタイトルバー テーマを指定する (ライブラリの責務)。
///
/// Slint の winit バックエンドは検出した色スキームで装飾テーマを固定する。
/// 内部 UI のスタイルを固定しているアプリでは、装飾も明示的に揃えないと
/// UI とタイトルバーの明暗が分裂する。winit ウィンドウは外観クエリ待ちで
/// 生成が遅れることがあるため、生成を待つ非同期タスクで適用する (生成済み
/// なら即時)。ウィンドウ生成直後に呼ぶ。
///
/// モーダル子ダイアログのテーマは modal-window が既定で親に揃えるため、
/// アプリが個別に指定する必要はない。
pub fn apply_titlebar_theme<P: slint::ComponentHandle + 'static>(
	component: &P,
	theme: TitlebarTheme,
) {
	use slint::winit_030::winit::window::Theme;
	use slint::winit_030::WinitWindowAccessor;
	let weak = component.as_weak();
	let _ = slint::spawn_local(async move {
		let Some(component) = weak.upgrade() else { return };
		let Ok(winit_window) = component.window().winit_window().await else { return };
		winit_window.set_theme(match theme {
			TitlebarTheme::Light => Some(Theme::Light),
			TitlebarTheme::Dark => Some(Theme::Dark),
			TitlebarTheme::System => None,
		});
	});
}

// The main window HWND, captured once the native window exists. Passing it as
// the owner to MessageBoxW makes the box application-modal: while it is up the
// parent window cannot be selected (clicking it just flashes/shakes, the stock
// Windows behaviour). Without an owner HWND the box degrades to modeless
// behaviour and the parent stays selectable.
#[cfg(windows)]
pub fn main_window_hwnd(ui: &slint::Window) -> windows_sys::Win32::Foundation::HWND {
	use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
	use slint::winit_030::WinitWindowAccessor;
	ui.with_winit_window(|w| match w.window_handle() {
		Ok(h) => match h.as_raw() {
			RawWindowHandle::Win32(h) => h.hwnd.get() as windows_sys::Win32::Foundation::HWND,
			_ => std::ptr::null_mut(),
		},
		Err(_) => std::ptr::null_mut(),
	})
	.unwrap_or(std::ptr::null_mut())
}

/// 実行ファイルに埋め込まれたアイコンレソース (`icon_resource_id` は
/// MAKEINTRESOURCE 解釈される序数) を、タイトルバー / タスクバーに出す
/// ウインドウアイコンとして設定する。winit が用意する既定アイコンではなく、
/// exe とまったく同じリソースを使う。
/// プロセスにつき一度だけ設定する (2 回目以降は何もしない)。
#[cfg(windows)]
pub fn install_window_icon(
	hwnd: windows_sys::Win32::Foundation::HWND,
	icon_resource_id: usize,
) {
	use windows_sys::Win32::Foundation::{LPARAM, WPARAM};
	use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		GetSystemMetrics, LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON,
		LR_DEFAULTCOLOR, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, WM_SETICON,
	};
	static WINDOW_ICON_SET: std::sync::atomic::AtomicBool =
		std::sync::atomic::AtomicBool::new(false);
	if WINDOW_ICON_SET.swap(true, std::sync::atomic::Ordering::SeqCst) {
		return;
	}
	let module = unsafe { GetModuleHandleW(std::ptr::null()) };
	if module.is_null() {
		return;
	}
	// MAKEINTRESOURCEW(icon_resource_id): リソース ID をそのままポインタ
	// として渡す (リソースの名前ではなく序数)。
	let name = icon_resource_id as *const u16;
	let (big, small) = unsafe {
		let (cx, cy) = (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON));
		let (sx, sy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
		(
			LoadImageW(module, name, IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR),
			LoadImageW(module, name, IMAGE_ICON, sx, sy, LR_DEFAULTCOLOR),
		)
	};
	if !big.is_null() {
		unsafe { SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, big as LPARAM); }
	}
	if !small.is_null() {
		unsafe { SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, small as LPARAM); }
	}
}

// When the window is not focused, winit stops reporting keyboard/modifier
// events while the mouse wheel still keeps arriving. Query the OS for the live
// shift/ctrl state so shift+wheel (horizontal) and ctrl+wheel (zoom) keep
// working even while the window is inactive.
#[cfg(windows)]
mod win_mods {
	#[link(name = "User32")]
	unsafe extern "system" {
		fn GetAsyncKeyState(vKey: i32) -> i16;
	}
	pub fn pressed(vkey: i32) -> bool {
		unsafe { GetAsyncKeyState(vkey) < 0 }
	}
}
#[cfg(windows)]
pub fn os_modifier_keys() -> (bool, bool) {
	(win_mods::pressed(0x10), win_mods::pressed(0x11)) // VK_SHIFT / VK_CONTROL
}
#[cfg(not(windows))]
pub fn os_modifier_keys() -> (bool, bool) {
	(false, false)
}

/// 当プロセスのトップレベルウィンドウ (自 pid) を列挙する (X11)。
/// `_NET_CLIENT_LIST` (EWMH) が無ければ root 直下の子も候補にする。
/// 順序は EWMH 提示順: winit のメイン窓が先頭、後から出したダイアログが
/// 後ろに並ぶ。カーソルの取り付け先とダイアログのモーダル化の両方で使う。
#[cfg(target_os = "linux")]
pub fn linux_own_windows(conn: &x11rb::rust_connection::RustConnection) -> Vec<u32> {
	use x11rb::connection::Connection;
	use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

	let root = conn.setup().roots[0].root;
	let pid = std::process::id() as u32;

	let clients = conn
		.intern_atom(false, b"_NET_CLIENT_LIST")
		.ok()
		.and_then(|c| c.reply().ok())
		.map(|r| r.atom);
	let wm_pid = conn
		.intern_atom(false, b"_NET_WM_PID")
		.ok()
		.and_then(|c| c.reply().ok())
		.map(|r| r.atom);

	let mut candidates: Vec<u32> = Vec::new();
	if let Some(ca) = clients {
		if let Some(rep) = conn
			.get_property(false, root, ca, AtomEnum::WINDOW, 0, u32::MAX)
			.ok()
			.and_then(|c| c.reply().ok())
		{
			candidates.extend(rep.value32().map(|it| it.collect::<Vec<u32>>()).unwrap_or_default());
		}
	}
	// EWMH を通らない環境用: root の直接の子も候補に加える。
	if let Some(qt) = conn.query_tree(root).ok().and_then(|c| c.reply().ok()) {
		candidates.extend(qt.children);
	}
	let pa = match wm_pid {
		Some(pa) => pa,
		None => return Vec::new(),
	};
	let mut own: Vec<u32> = Vec::new();
	for w in candidates {
		if own.contains(&w) {
			continue;
		}
		let Some(rep) = conn
			.get_property(false, w, pa, AtomEnum::CARDINAL, 0, 1)
			.ok()
			.and_then(|c| c.reply().ok())
		else {
			continue;
		};
		if rep.value32().is_some_and(|mut it| it.any(|v| v == pid)) {
			own.push(w);
		}
	}
	own
}