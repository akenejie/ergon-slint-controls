//! カスタムカーソル / 標準カーソルを各 OS のネイティブ API でウィンドウへ
//! 取り付ける汎用バックエンド。
//!
//! どのカーソルを出すか (ポリシー) と、その画像の生成は呼び出し側が決める。
//! このモジュールは「渡されたビットマップを OS カーソルとして載せる」「標準
//! カーソルへ戻す」処理だけを OS ごとに分岐して担う:
//! - Windows: DIBSection から HCURSOR を作り、wndproc をサブクラス化して
//!   WM_SETCURSOR で常に自前カーソルを再適用する。非クライアント領域の
//!   クリックは `take_non_client_click()` で取り出せる。
//! - Linux/X11: Render 拡張で ARGB32 のカーソルを作り、自プロセスの
//!   トップレベルウィンドウへ change_window_attributes で取り付ける。
//! - macOS / その他: winit の標準カーソルアイコンにフォールバックする
//!   (カスタムビットマップは非対応なので `set_custom_cursor` は false)。

/// 0xAARRGGBB (ネイティブエンディアン) のトップダウン・ピクセル列で表す
/// カーソル画像。`hotspot_*` は左上原点のピクセル座標。
pub struct CursorBitmap {
	pub width: u32,
	pub height: u32,
	pub hotspot_x: u32,
	pub hotspot_y: u32,
	pub pixels: Vec<u32>,
}

/// OS 標準カーソルの種別。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SystemCursor {
	Default,
	Crosshair,
}

// ---------------------------------------------------------------------------
// 非 Windows: カーソル適用先の winit ウィンドウと、現在のカーソル種別
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
use std::sync::Arc;
#[cfg(not(windows))]
use slint::winit_030::winit;

#[cfg(not(windows))]
static WINDOW: std::sync::OnceLock<Arc<winit::window::Window>> = std::sync::OnceLock::new();

/// カーソル適用先の winit ウィンドウを覚える。winit::Window は Clone できない
/// ので、同期アクセサが渡してくる Arc をそのまま保持する。
#[cfg(not(windows))]
pub fn cache_window(win: Arc<winit::window::Window>) {
	let _ = WINDOW.set(win);
}

// 現在ウィンドウに載っているカーソル種別 (重複適用を避けるため)。
// 0 = Default, 1 = Crosshair, 2 = Custom。
#[cfg(not(windows))]
static SYSTEM_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
#[cfg(not(windows))]
const STATE_DEFAULT: u8 = 0;
#[cfg(not(windows))]
const STATE_CROSSHAIR: u8 = 1;
#[cfg(not(windows))]
const STATE_CUSTOM: u8 = 2;

/// 標準カーソルへ切り替える。既に同じカーソルが載っていれば何もしない。
#[cfg(not(windows))]
pub fn set_system_cursor(kind: SystemCursor) {
	#[cfg(target_os = "linux")]
	free_x11_custom_cursor();
	let want = match kind {
		SystemCursor::Default => STATE_DEFAULT,
		SystemCursor::Crosshair => STATE_CROSSHAIR,
	};
	if SYSTEM_STATE.load(std::sync::atomic::Ordering::Relaxed) == want {
		return;
	}
	let Some(win) = WINDOW.get() else { return };
	win.set_cursor(match kind {
		SystemCursor::Default => winit::window::CursorIcon::Default,
		SystemCursor::Crosshair => winit::window::CursorIcon::Crosshair,
	});
	SYSTEM_STATE.store(want, std::sync::atomic::Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Linux/X11: Render の ARGB32 カスタムカーソル
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
use std::borrow::Cow;
#[cfg(target_os = "linux")]
use x11rb::connection::Connection as _;
#[cfg(target_os = "linux")]
use x11rb::image::{BitsPerPixel, Image, ImageOrder, ScanlinePad};
#[cfg(target_os = "linux")]
use x11rb::protocol::render::{self, PictType};
#[cfg(target_os = "linux")]
use x11rb::protocol::xproto::{self, ConnectionExt};
#[cfg(target_os = "linux")]
use x11rb::rust_connection::RustConnection;

#[cfg(target_os = "linux")]
thread_local! {
	static X11_CURSOR: std::cell::RefCell<Option<X11CursorState>> = const { std::cell::RefCell::new(None) };
	static ARGB32_FORMAT: std::cell::RefCell<Option<u32>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "linux")]
struct X11CursorState {
	conn: RustConnection,
	/// このキーで表されるカーソルが現在ウィンドウに載っている。
	key: i64,
	/// カーソルを載せたトップレベルウィンドウ (解放時に外すため)。
	target: u32,
	cursor: u32,
	pixmap: u32,
	gc: u32,
	picture: u32,
}

#[cfg(target_os = "linux")]
fn free_x11_ring(st: &X11CursorState) {
	// カーソルを載せたウィンドウから外す (FreeCursor だけではウィンドウに
	// 設定済みのカーソルは外れない)。
	let _ = xproto::change_window_attributes(
		&st.conn,
		st.target,
		&xproto::ChangeWindowAttributesAux { cursor: None, ..Default::default() },
	)
	.map(|c| c.ignore_error());
	let _ = render::free_picture(&st.conn, st.picture).map(|c| c.ignore_error());
	let _ = xproto::free_pixmap(&st.conn, st.pixmap).map(|c| c.ignore_error());
	let _ = xproto::free_gc(&st.conn, st.gc).map(|c| c.ignore_error());
	let _ = xproto::free_cursor(&st.conn, st.cursor).map(|c| c.ignore_error());
	let _ = st.conn.flush();
}

/// カスタムカーソルをウィンドウから外して解放する (標準カーソルへ戻すとき)。
#[cfg(target_os = "linux")]
fn free_x11_custom_cursor() {
	X11_CURSOR.with(|slot| {
		if let Some(old) = slot.borrow_mut().take() {
			free_x11_ring(&old);
		}
	});
}

/// ARGB32 ピクセルフォーマットの id を (キャッシュ付きで) 求める。
/// 条件: Direct color、depth 32、alpha 24bit shift、red 16bit shift。
#[cfg(target_os = "linux")]
fn argb32_format(conn: &RustConnection) -> Option<u32> {
	if let Some(f) = ARGB32_FORMAT.with(|f| *f.borrow()) {
		return Some(f);
	}
	let reply = render::query_pict_formats(conn).ok()?.reply().ok()?;
	for fmt in reply.formats {
		if fmt.type_ == PictType::DIRECT
			&& fmt.depth == 32
			&& fmt.direct.alpha_shift == 24
			&& fmt.direct.red_shift == 16
		{
			ARGB32_FORMAT.with(|f| *f.borrow_mut() = Some(fmt.id));
			return Some(fmt.id);
		}
	}
	None
}

/// 当プロセスのトップレベルウィンドウを探す (主: winit のメイン窓)。
#[cfg(target_os = "linux")]
fn find_own_window(conn: &RustConnection) -> Option<u32> {
	crate::platform::linux_own_windows(conn).into_iter().next()
}

/// ピクセル列から X11 のカスタムカーソルを作り、対象ウィンドウへ取り付ける。
/// X11 が使えない (Wayland 等)・ARGB32 非対応の場合は None (呼び出し側が
/// 標準カーソルへフォールバックする)。`build` は接続とフォーマットが確認
/// できた後にだけ呼ぶ (非対応環境で画像生成を無駄に走らせない)。
#[cfg(target_os = "linux")]
fn build_x11_cursor(key: i64, build: impl FnOnce() -> CursorBitmap) -> Option<X11CursorState> {
	let (conn, _screen) = x11rb::connect(None).ok()?;
	let root = conn.setup().roots[0].root;
	let target = find_own_window(&conn)?;
	let format = argb32_format(&conn)?;

	let bitmap = build();
	let (w, h) = (bitmap.width as u16, bitmap.height as u16);

	// サーバー native への変換は Image::put 内で行われる。ホストのバイト順に合せて
	// ARGB32 (LE マシンでは B,G,R,A の順) のバイト列を作る。
	let order = if cfg!(target_endian = "little") {
		ImageOrder::LsbFirst
	} else {
		ImageOrder::MsbFirst
	};
	let mut bytes = Vec::with_capacity(bitmap.pixels.len() * 4);
	for px in &bitmap.pixels {
		bytes.extend_from_slice(&px.to_ne_bytes());
	}
	let img = Image::new(
		w,
		h,
		ScanlinePad::Pad32,
		32,
		BitsPerPixel::B32,
		order,
		Cow::Owned(bytes),
	)
	.ok()?;

	let pixmap = conn.generate_id().ok()?;
	conn.create_pixmap(32, pixmap, root, w, h).ok().map(|c| c.ignore_error())?;
	let gc = conn.generate_id().ok()?;
	conn.create_gc(gc, pixmap, &Default::default()).ok().map(|c| c.ignore_error())?;
	img.put(&conn, pixmap, gc, 0, 0).ok()?;

	let picture = conn.generate_id().ok()?;
	render::create_picture(&conn, picture, pixmap, format, &Default::default())
		.ok()
		.map(|c| c.ignore_error())?;
	let cursor = conn.generate_id().ok()?;
	render::create_cursor(&conn, cursor, picture, bitmap.hotspot_x as u16, bitmap.hotspot_y as u16)
		.ok()
		.map(|c| c.ignore_error())?;

	// winit の接続とは別だが、SetWindowAttributes の cursor 指定は別接続からでも
	// 任意のウィンドウに対して行える。
	xproto::change_window_attributes(
		&conn,
		target,
		&xproto::ChangeWindowAttributesAux { cursor: Some(cursor), ..Default::default() },
	)
	.ok()
	.map(|c| c.ignore_error())?;
	conn.flush().ok()?;

	Some(X11CursorState { conn, key, target, cursor, pixmap, gc, picture })
}

/// `key` で識別されるカスタムカーソルを取り付ける。既に同じキーが載っていれば
/// 何もしない。X11 非対応なら false を返す (画像は生成しない)。
#[cfg(target_os = "linux")]
pub fn set_custom_cursor(key: i64, build: impl FnOnce() -> CursorBitmap) -> bool {
	X11_CURSOR.with(|slot| {
		let mut slot = slot.borrow_mut();
		if let Some(st) = slot.as_ref() {
			if st.key == key {
				SYSTEM_STATE.store(STATE_CUSTOM, std::sync::atomic::Ordering::Relaxed);
				return true;
			}
		}
		match build_x11_cursor(key, build) {
			Some(new) => {
				if let Some(old) = slot.take() {
					free_x11_ring(&old);
				}
				*slot = Some(new);
				SYSTEM_STATE.store(STATE_CUSTOM, std::sync::atomic::Ordering::Relaxed);
				true
			}
			None => {
				if let Some(old) = slot.take() {
					free_x11_ring(&old);
				}
				false
			}
		}
	})
}

/// macOS / その他: winit の標準カーソルしか使えないのでカスタムは非対応。
#[cfg(all(not(windows), not(target_os = "linux")))]
pub fn set_custom_cursor(_key: i64, _build: impl FnOnce() -> CursorBitmap) -> bool {
	false
}

// ---------------------------------------------------------------------------
// Windows: DIBSection から HCURSOR を作り、wndproc で強制適用する
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod win {
	use super::{CursorBitmap, SystemCursor};
	use crate::platform::main_window_hwnd;
	use std::cell::RefCell;
	use std::os::windows::ffi::OsStrExt;
	use std::path::Path;
	use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
	use windows_sys::Win32::Graphics::Gdi::{
		BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS,
		DeleteObject, RGBQUAD,
	};
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		CallWindowProcW, CreateIconIndirect, DestroyIcon, GetWindowLongPtrW, GWLP_WNDPROC, HCURSOR,
		HTCLIENT, ICONINFO, IDC_ARROW, IDC_CROSS, LoadCursorFromFileW, LoadCursorW, SetCursor,
		SetWindowLongPtrW, WM_NCLBUTTONDOWN, WM_NCMBUTTONDOWN, WM_NCRBUTTONDOWN, WM_SETCURSOR,
		WNDPROC,
	};

	#[derive(Clone, Copy, PartialEq, Eq)]
	enum WinCursor {
		Default,
		Crosshair,
		Custom(i64),
	}

	thread_local! {
		static CURRENT: RefCell<WinCursor> = const { RefCell::new(WinCursor::Default) };
		static ORIG_WNDPROC: RefCell<Option<WNDPROC>> = const { RefCell::new(None) };
		static OVERRIDE_CURSOR: RefCell<Option<HCURSOR>> = const { RefCell::new(None) };
		static CACHE: RefCell<Vec<(i64, HCURSOR)>> = const { RefCell::new(Vec::new()) };
	}

	static OVERRIDE_PROC_INSTALLED: std::sync::atomic::AtomicBool =
		std::sync::atomic::AtomicBool::new(false);
	static NON_CLIENT_CLICK: std::sync::atomic::AtomicBool =
		std::sync::atomic::AtomicBool::new(false);

	// winit は設定したカーソルを WM_SETCURSOR ごとに再適用するため、素の
	// SetCursor は次のマウス移動で上書きされる。そこでウィンドウ手続きを
	// サブクラス化し、クライアント領域では常に自前カーソルを強制する。
	unsafe extern "system" fn override_wnd_proc(
		hwnd: HWND,
		msg: u32,
		wparam: WPARAM,
		lparam: LPARAM,
	) -> LRESULT {
		// タイトルバーなどクライアント外のクリックを呼び出し側へ通知する
		// (take_non_client_click で消費)。
		if msg == WM_NCLBUTTONDOWN || msg == WM_NCRBUTTONDOWN || msg == WM_NCMBUTTONDOWN {
			NON_CLIENT_CLICK.store(true, std::sync::atomic::Ordering::Relaxed);
		}
		if msg == WM_SETCURSOR && (lparam as u32 & 0xFFFF) == HTCLIENT {
			if let Some(h) = OVERRIDE_CURSOR.with(|c| *c.borrow()) {
				unsafe { SetCursor(h); }
				return 0;
			}
		}
		let orig = ORIG_WNDPROC.with(|c| c.borrow().clone()).unwrap_or(None);
		unsafe { CallWindowProcW(orig, hwnd, msg, wparam, lparam) }
	}

	/// ウィンドウ手続きをサブクラス化する (プロセスにつき一度だけ)。
	pub fn install_override(ui: &slint::Window) {
		if OVERRIDE_PROC_INSTALLED.load(std::sync::atomic::Ordering::SeqCst) {
			return;
		}
		let hwnd = main_window_hwnd(ui);
		if hwnd.is_null() {
			return;
		}
		if OVERRIDE_PROC_INSTALLED.swap(true, std::sync::atomic::Ordering::SeqCst) {
			return;
		}
		let orig = unsafe { GetWindowLongPtrW(hwnd, GWLP_WNDPROC) };
		ORIG_WNDPROC.with(|c| {
			*c.borrow_mut() = Some(unsafe { std::mem::transmute::<isize, WNDPROC>(orig) })
		});
		unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, override_wnd_proc as *const () as isize); }
	}

	/// 非クライアント領域クリックの通知を消費する。
	pub fn take_non_client_click() -> bool {
		NON_CLIENT_CLICK.swap(false, std::sync::atomic::Ordering::Relaxed)
	}

	fn override_to(h: HCURSOR, state: WinCursor) {
		OVERRIDE_CURSOR.with(|c| *c.borrow_mut() = Some(h));
		CURRENT.with(|c| *c.borrow_mut() = state);
		unsafe { SetCursor(h); }
	}

	fn insert_cache(key: i64, h: HCURSOR) {
		CACHE.with(|c| {
			let mut c = c.borrow_mut();
			c.insert(0, (key, h));
			while c.len() > 16 {
				let (_, old) = c.pop().unwrap();
				unsafe { DestroyIcon(old as *mut core::ffi::c_void); }
			}
		});
	}

	fn cached(key: i64) -> Option<HCURSOR> {
		CACHE.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, h)| *h))
	}

	/// ビットマップから HCURSOR を作る (DIBSection + 1bpp マスク)。
	fn build_hcursor(b: &CursorBitmap) -> Option<HCURSOR> {
		let dim = b.width as i32;
		let hdr = BITMAPINFOHEADER {
			biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
			biWidth: dim,
			biHeight: -(b.height as i32),
			biPlanes: 1,
			biBitCount: 32,
			biCompression: BI_RGB,
			..Default::default()
		};
		let bmi = BITMAPINFO { bmiHeader: hdr, bmiColors: [RGBQUAD::default()] };
		let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
		let hbm_color = unsafe {
			CreateDIBSection(std::ptr::null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0)
		};
		if hbm_color.is_null() {
			return None;
		}
		unsafe { std::ptr::copy_nonoverlapping(b.pixels.as_ptr(), bits as *mut u32, b.pixels.len()); }
		let stride = ((dim + 31) / 32) * 4;
		let mask = vec![0u8; (stride * b.height as i32) as usize];
		let hbm_mask = unsafe {
			CreateBitmap(dim, b.height as i32, 1, 1, mask.as_ptr() as *const core::ffi::c_void)
		};
		if hbm_mask.is_null() {
			unsafe { DeleteObject(hbm_color as *mut core::ffi::c_void); }
			return None;
		}
		let info = ICONINFO {
			fIcon: 0,
			xHotspot: b.hotspot_x,
			yHotspot: b.hotspot_y,
			hbmMask: hbm_mask,
			hbmColor: hbm_color,
		};
		let hic = unsafe { CreateIconIndirect(&info) };
		unsafe {
			DeleteObject(hbm_color as *mut core::ffi::c_void);
			DeleteObject(hbm_mask as *mut core::ffi::c_void);
		}
		if hic.is_null() { None } else { Some(hic as HCURSOR) }
	}

	/// `key` で識別されるカスタムカーソルを取り付ける。同じキーが載っていれば
	/// 何もしない (再生成しない)。生成に失敗したら false。
	pub fn set_custom_cursor(key: i64, build: impl FnOnce() -> CursorBitmap) -> bool {
		if CURRENT.with(|c| *c.borrow()) == WinCursor::Custom(key) {
			return true;
		}
		if let Some(h) = cached(key) {
			override_to(h, WinCursor::Custom(key));
			return true;
		}
		let Some(h) = build_hcursor(&build()) else { return false };
		insert_cache(key, h);
		override_to(h, WinCursor::Custom(key));
		true
	}

	/// `.cur` ファイルのカーソルを取り付ける (同じキーが載っていれば何もしない)。
	pub fn set_file_cursor(key: i64, path: &Path) -> bool {
		if CURRENT.with(|c| *c.borrow()) == WinCursor::Custom(key) {
			return true;
		}
		if let Some(h) = cached(key) {
			override_to(h, WinCursor::Custom(key));
			return true;
		}
		let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
		let h = unsafe { LoadCursorFromFileW(wide.as_ptr()) };
		if h.is_null() {
			return false;
		}
		insert_cache(key, h);
		override_to(h, WinCursor::Custom(key));
		true
	}

	/// 標準カーソルへ切り替える (重複適用しない)。
	pub fn set_system_cursor(kind: SystemCursor) {
		let (want, forced) = match kind {
			SystemCursor::Default => (WinCursor::Default, None),
			SystemCursor::Crosshair => {
				let cross = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_CROSS) };
				(WinCursor::Crosshair, (!cross.is_null()).then_some(cross))
			}
		};
		if CURRENT.with(|c| *c.borrow()) == want {
			return;
		}
		OVERRIDE_CURSOR.with(|c| *c.borrow_mut() = forced);
		CURRENT.with(|c| *c.borrow_mut() = want);
		let h = forced.unwrap_or_else(|| unsafe { LoadCursorW(std::ptr::null_mut(), IDC_ARROW) });
		unsafe { SetCursor(h); }
	}
}

#[cfg(windows)]
pub use win::{install_override, set_custom_cursor, set_file_cursor, set_system_cursor, take_non_client_click};

#[cfg(not(windows))]
pub fn install_override(_ui: &slint::Window) {}

#[cfg(not(windows))]
pub fn take_non_client_click() -> bool {
	false
}
