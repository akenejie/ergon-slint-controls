//! ネイティブプラットフォーム操作の汎用ヘルパ:
//! ウインドウハンドル取得、ウインドウアイコンの設定、モディファイアキーの照会。
//! アイコンは実行ファイルに埋め込まれたリソース (アイコンリソース ID は
//! 呼び出し側が渡す) を使うため、このクレートは実アイコンを持たない。

#[cfg(windows)]
pub fn encode_wide(s: &str) -> Vec<u16> {
	s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------------------
// Linux/X11: winit ウィンドウから X ウィンドウ ID (XID) を取得する互換層
// ---------------------------------------------------------------------------

/// X11 のウィンドウ ID を winit ウィンドウから取得する (フォーク互換層)。
///
/// crates.io の winit 0.30 には `Window::xlib_window()` が存在しない (0.29
/// で廃止され、`WindowExtX11` は空トレイト)。一方、modal-dialog feature が
/// 要求する winit パッチ フォーク (slint-modal-winit) は同名の inherent メ
/// ソッドを独自に提供している。Rust のメソッド解決は inherent を trait よ
/// り優先するため、フォーク使用時はそちら (実 XID)、非使用時 (crates.io 版
/// winit — `cargo publish` の tarball 検証ビルドなど、[patch] はパッケージ
/// に同梱されない) はこのトレイトのフォールバック (None) が選ばれ、どちら
/// でもコンパイルが通る。None 時の動作は各呼び出し側が既に持っているフォー
/// ラック (XID 未設定のまま続行) に委ねる。
#[cfg(all(unix, not(target_os = "macos")))]
// フォーク使用時は inherent メソッドが解決されるため本トレイトは未使用扱い
// になる (dead_code)。フォークなしビルド (tarball 検証) でのみ使用される。
#[allow(dead_code)]
pub(crate) trait WinitXlibWindow {
	/// X11 ウィンドウ ID。取得できない環境 (フォークなしの winit 0.30) では
	/// `None`。
	fn xlib_window(&self) -> Option<u32>;
}

#[cfg(all(unix, not(target_os = "macos")))]
impl WinitXlibWindow for slint::winit_030::winit::window::Window {
	fn xlib_window(&self) -> Option<u32> {
		None
	}
}

/// ウィンドウ装飾 (タイトルバー) のテーマ。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TitlebarTheme {
	Light,
	Dark,
	/// システム既定に従う (winit へは `None` を渡す)。
	System,
}

// setup_window で明示指定したテーマ。winit の Window::theme() は
// X11 では XSETTINGS 依存で None を返すことがあり、ダイアログの「親への
// 追従」の真実としては使えないため、指定値をこちらで保持する。
// 0 = System / 1 = Light / 2 = Dark
static APPLIED_TITLEBAR_THEME: std::sync::atomic::AtomicU8 =
	std::sync::atomic::AtomicU8::new(0);

/// [`setup_window`] で明示指定したテーマ (未指定なら `None`)。
/// modal_dialog が子ダイアログを親に揃えるのに使う。
pub fn applied_titlebar_theme() -> Option<TitlebarTheme> {
	use std::sync::atomic::Ordering::Relaxed;
	match APPLIED_TITLEBAR_THEME.load(Relaxed) {
		1 => Some(TitlebarTheme::Light),
		2 => Some(TitlebarTheme::Dark),
		_ => None,
	}
}

/// ウィンドウの初期設定を適用する (ライブラリの責務)。
///
/// - **タイトルバー テーマ**: Slint の winit バックエンドは検出した色スキー
///   ムで装飾テーマを固定する。内部 UI のスタイルを固定しているアプリでは、
///   装飾も明示的に揃えないと UI とタイトルバーの明暗が分裂する。
/// - **ユーザー操作時刻 (X11)**: `_NET_WM_USER_TIME` を設定する。EWMH の
///   フォーカス盗み取り防止はこのプロパティを見るため、これが無いと WM は
///   「ユーザー操作と無関係に開いた窓」として扱い、以後開くモーダル子ダイア
///   ログの親基準配置・フォーカス付与が行われない (winit はこのプロパティを
///   扱わないため、ライブラリが X プロトコルで直接設定する)。
///
/// winit ウィンドウは外観クエリ待ちで生成が遅れることがあるため、生成を待
/// つ非同期タスクで適用する (生成済みなら即時)。ウィンドウ生成直後に呼ぶ。
///
/// モーダル子ダイアログのテーマは modal_dialog が既定で親に揃えるため、
/// アプリが個別に指定する必要はない。子ダイアログのユーザー操作時刻も
/// modal_dialog が親の値から設定する。
pub fn setup_window<P: slint::ComponentHandle + 'static>(
	component: &P,
	theme: TitlebarTheme,
) {
	// メニューバー / メニューのテーマ馴染ませ (Linux は GTK テーマから取得)。
	// 対象の実体は MenuBarTheme グローバルの from_library 共有分で、ルート
	// コンポーネントの生成時 (共有グローバル群の生成時) にライブラリへ通知
	// 済み (lib.rs の ergon_slint_controlsSharedGlobals ラッパー参照)。メイン
	// スレッドで GTK を初期化して色を取得し、登録済みの全実体へ流し込む。
	// Windows / macOS では何もしない (既定色のまま)。
	#[cfg(target_os = "linux")]
	crate::menu_bar_theme::apply_computed();
	// (UI フォントはオプトイン: ui_font::apply_os_default() をアプリが呼ん
	// だときだけ、shown フックで各ウィンドウの既定フォントを OS 標準へ揃え
	// る。呼ばないときは機能ごと最終バイナリから外れる。)
	use slint::winit_030::winit::window::Theme;
	use slint::winit_030::WinitWindowAccessor;
	use std::sync::atomic::Ordering::Relaxed;
	APPLIED_TITLEBAR_THEME.store(
		match theme {
			TitlebarTheme::System => 0,
			TitlebarTheme::Light => 1,
			TitlebarTheme::Dark => 2,
		},
		Relaxed,
	);
	let weak = component.as_weak();
	let _ = slint::spawn_local(async move {
		let Some(component) = weak.upgrade() else { return };
		let Ok(winit_window) = component.window().winit_window().await else { return };
		winit_window.set_theme(match theme {
			TitlebarTheme::Light => Some(Theme::Light),
			TitlebarTheme::Dark => Some(Theme::Dark),
			TitlebarTheme::System => None,
		});
		// ウィンドウ サービスの準備 (OS ごと。アプリ側は OS 分岐を書かない):
		// - Windows: モーダル ダイアログの親 HWND の確保 + WM_SETCURSOR で
		//   自前カーソルを維持する wndproc サブクラス
		// - Linux / macOS: カーソル制御用の winit ウィンドウ (Arc) のキャッシュ
		crate::dialogs::capture_dialog_owner(&component.window());
		crate::cursor::install_override(&component.window());
		#[cfg(not(windows))]
		crate::cursor::cache_window(winit_window.clone());
		#[cfg(windows)]
		let _ = winit_window;
		// ユーザー操作時刻 (X11)。winit が扱わないため X プロトコルで直接
		// 設定する。X プロトコルには「現在時刻」を問い合わせる API が無いた
		// め、PropertyNotify トリックで取得する。EWMH のフォーカス盗み取り防
		// 止はこの値を見るため、メイン窓のオープン時点で WM へユーザー操作の
		// 時刻が伝わる。以後に開かれる子ダイアログの親基準配置・フォーカス付
		// 与の土台になる。
		#[cfg(target_os = "linux")]
		{
			match winit_window.xlib_window() {
				Some(xid) => {
					let xid = xid as u32;
					if let Some(t) = crate::gtk::current_server_time(xid) {
						crate::gtk::write_user_time(xid, t);
					}
				}
				None => {}
			}
		}
	});
}

/// カーソル追従ツールチップの 1 軸分の配置を決める (SharedGlobals::new が
/// TooltipHub.place のハンドラから呼ぶ)。
///
/// ポインタ位置を基準に、パネルを右 / 下へ出せるならそこへ出す (ポインタ =
/// パネルの左上 / 左下角)。はみ出すなら左 / 上へ反転して (ポインタ = 右上 /
/// 右下角)、左右または上下のどちらに置いてもはみ出す場合だけ例外で、ウィン
/// ドウ内に収める (クランプ)。隙間は設けない (ポインタが角に密着)。
pub(crate) fn place_tooltip_axis(cursor: f32, size: f32, limit: f32) -> f32 {
	if cursor + size <= limit {
		// 右 / 下へ出して収まる。
		cursor
	} else if cursor - size >= 0.0 {
		// 右 / 下にはみ出すが、左 / 上へ出せば収まる (反転)。
		cursor - size
	} else {
		// どちらに置いてもはみ出す (例外): ウィンドウ内に収める。
		let hi = (limit - size).max(0.0);
		cursor.min(hi).max(0.0)
	}
}

/// ショートカット表記の OS 差異を吸収する (macOS: Ctrl → Cmd)。アプリは
/// Windows 記法 ("Ctrl+Z") を 1 回書くだけでよい。
pub fn shortcut_label(text: &str) -> String {
	#[cfg(target_os = "macos")]
	{
		text.replace("Ctrl", "Cmd")
	}
	#[cfg(not(target_os = "macos"))]
	{
		text.to_string()
	}
}

// The main window HWND, captured once the native window exists. Passing it as
// the owner to MessageBoxW makes the box application-modal: while it is up the
// parent window cannot be selected (clicking it just flashes/shakes, the stock
// Windows behaviour). Without an owner HWND the box degrades to modeless
// behaviour and the parent stays selectable.
#[cfg(windows)]
pub fn window_hwnd(ui: &slint::Window) -> windows_sys::Win32::Foundation::HWND {
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

/// アプリ アイコンの供給。アプリは手持ちの情報だけを入れる (OS 分岐は
/// [`apply_app_icon`](platform::apply_app_icon) が担う)。
#[derive(Default)]
pub struct AppIcon {
	/// Windows: 実行ファイルに埋め込んだアイコン リソースの ID
	/// (build.rs の `1 ICON "icon.ico"` の 1)。
	pub windows_resource_id: Option<usize>,
	/// Linux / macOS: アイコン画像 (幅 × 高さの RGBA ピクセル、行優先)。
	/// 用意できない (画像が無い / デコードに失敗した) 場合は None でよく、
	/// その OS では既定アイコンのままになる。
	pub image: Option<(Vec<u8>, u32, u32)>,
}

/// アプリ アイコンを OS ごとの正しい方法で適用する。アプリはアイコンを用意
/// して起動時に 1 回呼ぶだけ (OS 分岐なし)。
///
/// - **Windows**: 実行ファイルに埋め込んだリソースをタイトルバー / タスク
///   バーへ設定する (install_window_icon)。
/// - **Linux / macOS**: RGBA 画像を winit ウィンドウへ設定する。
///
/// winit ウィンドウの生成を待つ必要があるため、内部で非同期タスクを起こす
/// (呼び出しは即座に返る)。アイコンが用意できていない側の OS では何もしな
/// い (エラーにはならない。じかん君のようにアイコンを定義していないアプリ
/// もそのまま呼んでよい)。
///
/// ```ignore
/// platform::apply_app_icon(&ui, AppIcon {
///     windows_resource_id: Some(1),
///     image: load_icon_rgba(),   // 失敗したら None
/// });
/// ```
pub fn apply_app_icon<C: slint::ComponentHandle + 'static>(ui: &C, icon: AppIcon) {
	let weak = ui.as_weak();
	let _ = slint::spawn_local(async move {
		use slint::winit_030::WinitWindowAccessor;
		let Some(ui) = weak.upgrade() else { return };
		let Ok(winit_window) = ui.window().winit_window().await else { return };
		#[cfg(windows)]
		if let Some(id) = icon.windows_resource_id {
			let hwnd = window_hwnd(&ui.window());
			if !hwnd.is_null() {
				install_window_icon(hwnd, id);
			}
		}
		#[cfg(not(windows))]
		if let Some((rgba, width, height)) = icon.image {
			if let Ok(winit_icon) = slint::winit_030::winit::window::Icon::from_rgba(
				rgba, width, height,
			) {
				winit_window.set_window_icon(Some(winit_icon));
			}
		}
		let _ = winit_window; // Windows では未使用になるのを防ぐ
	});
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