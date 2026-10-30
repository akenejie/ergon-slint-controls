//! クロスプラットフォームのネイティブダイアログ実装:
//! - 情報 / エラーのメッセージ表示
//! - Yes/No などの確認
//! - 任意ラベル・既定ボタンを指定できる複数ボタンのプロンプト
//! - フォルダ選択
//! 加えて、モーダルダイアログの表示中に親ウィンドウへ溜まった入力
//! (phantom click) を捨てる仕組みを提供する。
//!
//! タイトル・本文・ボタン文字列は呼び出し側が渡す。このモジュールはそれらを
//! 各 OS のネイティブ API (Windows: MessageBoxW / TaskDialogIndirect /
//! IFileOpenDialog、Linux: zenity / kdialog / xmessage、macOS: osascript) で
//! 描画するだけに徹する。

use std::path::{Path, PathBuf};

#[cfg(windows)]
use crate::platform::encode_wide;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
#[cfg(windows)]
use windows_sys::Win32::UI::WindowsAndMessaging::{
	CallNextHookEx, GetClassNameW, GetWindowRect, LoadIconW, MessageBoxW, SetWindowPos,
	SetWindowsHookExW, UnhookWindowsHookEx, HCBT_ACTIVATE, HHOOK, IDCANCEL, IDI_INFORMATION,
	IDNO, IDOK, IDYES, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, SWP_NOACTIVATE, SWP_NOSIZE,
	SWP_NOZORDER, WH_CBT,
};

// The main window HWND, captured once the native window exists. Passing it as
// the owner to MessageBoxW makes the box application-modal: while it is up the
// parent window cannot be selected (clicking it just flashes/shakes, the stock
// Windows behaviour). Without an owner HWND the box degrades to modeless
// behaviour and the parent stays selectable.
#[cfg(windows)]
static DIALOG_OWNER: std::sync::atomic::AtomicPtr<core::ffi::c_void> =
	std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// 初期化済みなら親ウィンドウの HWND を覚え、モーダルダイアログの親に使う。
/// 一度だけ設定する (ウィンドウが再生成されても最初の HWND を維持)。
#[cfg(windows)]
pub fn capture_dialog_owner(ui: &slint::Window) {
	let hwnd = crate::platform::main_window_hwnd(ui);
	if !hwnd.is_null() {
		let _ = DIALOG_OWNER.compare_exchange(
			std::ptr::null_mut(),
			hwnd,
			std::sync::atomic::Ordering::Relaxed,
			std::sync::atomic::Ordering::Relaxed,
		);
	}
}

/// TaskDialogIndirect の共通呼び出し。`hwndParent` (DIALOG_OWNER = 親ウィンドウ)
/// を渡すと、OS がタスクダイアログを「親ウィンドウの中央」に配置する
/// (TaskDialog の既定動作。TDF_POSITION_RELATIVE_TO_WINDOW を付けない場合)。
/// 位置はアプリ側で一切計算しない。押されたボタンの ID を返す。
///
/// メッセージ本文は `pszContent` (本文用 = 黒字) に渡す。`pszMainInstruction`
/// は TaskDialog の「大きな見出し」用のスロットで、テーマによって青字で描かれる。
/// そこに本文を渡すと全体が青い大きな文字になってしまうため、ここは空のままに
/// して本文を黒字の detail スロットへ入れる。
#[cfg(windows)]
fn task_dialog_center_parent(
	title: &str,
	message: &str,
	icon: *mut core::ffi::c_void,
	common_buttons: i32,
	default_button: i32,
) -> i32 {
	use windows_sys::Win32::UI::Controls::{
		TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDF_ALLOW_DIALOG_CANCELLATION,
		TDF_POSITION_RELATIVE_TO_WINDOW, TDF_USE_HICON_MAIN,
	};
	let title_w = encode_wide(title);
	let message_w = encode_wide(message);
	let mut config = TASKDIALOGCONFIG::default();
	config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
	config.hwndParent = DIALOG_OWNER.load(std::sync::atomic::Ordering::Relaxed);
	// TDF_POSITION_RELATIVE_TO_WINDOW が「hwndParent 基準でセンタリング」を
	// 有効にする。これを付けない場合、OS は hwndParent を渡していても
	// 「モニタ基準」でダイアログを配置する (画面中央になる)。
	config.dwFlags =
		TDF_POSITION_RELATIVE_TO_WINDOW | TDF_USE_HICON_MAIN | TDF_ALLOW_DIALOG_CANCELLATION;
	config.pszWindowTitle = title_w.as_ptr();
	// 大きな青字の見出しスロットは空、本文は黒字の detail スロットへ。
	config.pszMainInstruction = std::ptr::null();
	config.pszContent = message_w.as_ptr();
	config.Anonymous1 = TASKDIALOGCONFIG_0 { hMainIcon: icon };
	config.dwCommonButtons = common_buttons;
	config.nDefaultButton = default_button;
	let mut result = 0i32;
	let hr = unsafe {
		TaskDialogIndirect(
			&config,
			&mut result,
			std::ptr::null_mut(),
			std::ptr::null_mut(),
		)
	};
	if hr == 0 {
		result
	} else {
		default_button
	}
}

/// エラー / 情報のネイティブメッセージボックスのアイコン種別。エラーと情報で
/// 関数を分けず、この種別だけを切り替えて 1 つの関数に統一する。
#[derive(Clone, Copy)]
pub enum MessageIcon {
	/// 赤い × (従来の MB_ICONERROR と同じ)。
	Error,
	/// 青い i (従来の MB_ICONINFORMATION と同じ)。
	Info,
}

// MessageBoxW は親 HWND を渡しても「画面の作業領域の中央」に出るため、
// ダイアログが作られた瞬間 (CBT フック) に親ウィンドウの中央へ移動する
// (`center_message_box` 参照)。
#[cfg(windows)]
thread_local! {
	static MSGBOX_CENTER_STATE: std::cell::Cell<Option<(HHOOK, HWND)>> =
		const { std::cell::Cell::new(None) };
}

#[cfg(windows)]
unsafe extern "system" fn message_box_center_hook(
	code: i32,
	w_param: WPARAM,
	l_param: LPARAM,
) -> LRESULT {
	// チェーンの次のフックへ必ず渡す (返り値 0 以外は操作をキャンセルする)。
	let hook = MSGBOX_CENTER_STATE.with(|s| s.get()).map(|(h, _)| h);
	let next = unsafe {
		CallNextHookEx(hook.unwrap_or(std::ptr::null_mut()), code, w_param, l_param)
	};
	if code != HCBT_ACTIVATE as i32 {
		return next;
	}
	// MessageBoxW が作る #32770 ダイアログだけを移動対象にする。それ以外の
	// ウィンドウが先にアクティブになっても (通常は起こらない) フックは張った
	// ままにして、後続のアクティブで移動できるようにする。
	if let Some((hook, owner)) = MSGBOX_CENTER_STATE.with(|s| s.get()) {
		let hwnd = w_param as HWND;
		if message_box_class(hwnd) {
			MSGBOX_CENTER_STATE.with(|s| s.set(None));
			center_message_box(hwnd, owner);
			// 移動し終わったらフックを外す (MessageBoxW が戻るまで張っておくと
			// 同じダイアログへ繰り返しヒットしてしまうため)。
			let _ = unsafe { UnhookWindowsHookEx(hook) };
		}
	}
	next
}

#[cfg(windows)]
fn message_box_class(hwnd: HWND) -> bool {
	// 標準メッセージボックスはダイアログ (#32770) クラスで作られる。
	let mut buf = [0u16; 64];
	let len = unsafe { GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
	let name = &buf[..(len.max(0) as usize).min(buf.len())];
	// "#32770" は UTF-16 でも 1 文字ずつ ASC のまま。
	const DIALOG_CLASS: [u16; 6] = [
		b'#' as u16, b'3' as u16, b'2' as u16, b'7' as u16, b'7' as u16, b'0' as u16,
	];
	name.iter().eq(&DIALOG_CLASS)
}

#[cfg(windows)]
fn center_message_box(dlg: HWND, owner: HWND) {
	let mut dr = std::mem::MaybeUninit::<RECT>::uninit();
	let mut or = std::mem::MaybeUninit::<RECT>::uninit();
	let ok = unsafe {
		GetWindowRect(dlg, dr.as_mut_ptr()) != 0 && GetWindowRect(owner, or.as_mut_ptr()) != 0
	};
	let (dr, or) = unsafe { (dr.assume_init(), or.assume_init()) };
	if !ok || dr.right == dr.left || or.right == or.left {
		return;
	}
	// 親ウィンドウの中央にダイアログが来る位置を計算し、サイズ/前面順は
	// 変えずに移動だけする。
	let x = or.left + (or.right - or.left - (dr.right - dr.left)) / 2;
	let y = or.top + (or.bottom - or.top - (dr.bottom - dr.top)) / 2;
	unsafe {
		SetWindowPos(
			dlg,
			std::ptr::null_mut(),
			x,
			y,
			0,
			0,
			SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
		);
	}
}

/// エラー / 情報のネイティブメッセージボックス (Windows)。
///
/// 以前は TaskDialogIndirect を使っていたが、TaskDialog は本文を黒字の
/// detail スロットに入れても「大きな見出し + 固定幅」の独特なレイアウトに
/// なり、一般的なメッセージボックスの見た目と違っていた。ここでは OS 標準の
/// MessageBoxW を使うことで、本文の長さに応じてサイズが自動で決まり、OK
/// ボタンも標準サイズになる。アイコンだけを種別で差し替える。
///
/// fire-and-forget。親ウィンドウの HWND を owner に渡すので、OS 側でモーダルに
/// なる (呼び出し側のスレッドは問わない)。
#[cfg(windows)]
pub fn show_message_box(title: &str, text: &str, kind: MessageIcon) {
	let title = title.to_owned();
	let text = text.to_owned();
	let _ = slint::invoke_from_event_loop(move || {
		let title_w = encode_wide(&title);
		let text_w = encode_wide(&text);
		let icon = match kind {
			MessageIcon::Error => MB_ICONERROR,
			MessageIcon::Info => MB_ICONINFORMATION,
		};
		let owner = DIALOG_OWNER.load(std::sync::atomic::Ordering::Relaxed);
		unsafe {
			// 親ウィンドウ中央への移動用に、このスレッド限定の CBT フックを
			// MessageBoxW の間だけ張る。フックプロシージャ内で移動してから
			// 自動的に外れる。
			let hook =
				SetWindowsHookExW(WH_CBT, Some(message_box_center_hook), std::ptr::null_mut(), GetCurrentThreadId());
			let hook_active = !hook.is_null();
			if hook_active {
				MSGBOX_CENTER_STATE.with(|s| s.set(Some((hook, owner))));
			}
			MessageBoxW(owner, text_w.as_ptr(), title_w.as_ptr(), MB_OK | icon);
			if hook_active {
				let _ = UnhookWindowsHookEx(hook);
				MSGBOX_CENTER_STATE.with(|s| s.set(None));
			}
		}
	});
}

// 表示中のネイティブダイアログ数。0 以外なら親ウィンドウは入力を受け付けない。
// Windows は TaskDialogIndirect に親 HWND を渡して OS 側に grab を任せているが、
// Linux/macOS のダイアログは別プロセスなので自前で入力を止める必要がある。
// 呼び出し側は winit のイベントフィルタなどから `dialog_active()` を参照する。
#[cfg(not(windows))]
static DIALOG_ACTIVE: std::sync::atomic::AtomicUsize =
	std::sync::atomic::AtomicUsize::new(0);

#[cfg(not(windows))]
fn dialog_enter() {
	DIALOG_ACTIVE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(not(windows))]
fn dialog_leave() {
	let _ = DIALOG_ACTIVE.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

/// モーダルダイアログ表示中であることを親ウィンドウへ通知する (入力遮断)。
/// 外部プロセス (zenity 等) に加えて、同一プロセスの Slint 製ダイアログ
/// (アプリ側の `dialog` モジュール) が表示・非表示を切り替えるたびに呼ぶ。
#[cfg(not(windows))]
pub fn dialog_push() {
	dialog_enter();
}

#[cfg(not(windows))]
pub fn dialog_pop() {
	dialog_leave();
}

/// ネイティブダイアログが表示中かどうか。表示中なら親ウィンドウの入力を捨てる
/// (Windows は OS 側 grab があるので対象外)。
#[cfg(not(windows))]
pub fn dialog_active() -> bool {
	DIALOG_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) != 0
}

// 同期ダイアログ (zenity / kdialog / osascript) は `.status()` / `.output()` で
// UI スレッドを停止する。そのあいだに親ウィンドウへ届いた入力は winit のキュー
// に残り、ダイアログが閉じた直後に「phantom click」として再生される。
// winit にはキューを直接 drain する API が無いので、ダイアログから戻った直後の
// 数ターンだけ入力を捨てる。Windows は OS 側 grab があるので対象外。
//
// 解除はイベントループへ user event を投げて行う。バックエンドにより OS イベント
// を先にディスパッチする X11 と、user event を先にディスパッチする Wayland で
// 順序が逆なので、1 ターンではなく STALE_INPUT_TURNS_TO_DROP ターン保持する。
#[cfg(not(windows))]
static STALE_INPUT_TURNS: std::sync::atomic::AtomicUsize =
	std::sync::atomic::AtomicUsize::new(0);

#[cfg(not(windows))]
const STALE_INPUT_TURNS_TO_DROP: usize = 3;

// 解除イベントが万一届かない場合 (イベントループの外から呼ばれた等) に、
// 入力が永久に.drop され続けるのを防ぐ保険。期限切れなら強制的に打ち切る。
#[cfg(not(windows))]
const STALE_INPUT_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

#[cfg(not(windows))]
static STALE_INPUT_DEADLINE: std::sync::Mutex<Option<std::time::Instant>> =
	std::sync::Mutex::new(None);

/// 同期ダイアログが閉じた直後の滞留入力破棄中かどうか。親ウィンドウへ届いた
/// winit イベントを 数ターン捨てるために使う。
#[cfg(not(windows))]
pub fn stale_input_pending() -> bool {
	if STALE_INPUT_TURNS.load(std::sync::atomic::Ordering::Relaxed) == 0 {
		return false;
	}
	let Ok(mut deadline) = STALE_INPUT_DEADLINE.lock() else {
		return false;
	};
	match *deadline {
		Some(end) if std::time::Instant::now() < end => true,
		_ => {
			*deadline = None;
			STALE_INPUT_TURNS.store(0, std::sync::atomic::Ordering::Relaxed);
			false
		}
	}
}

/// UI スレッドを止める同期ダイアログの呼び出しを挟むラッパ。ダイアログが返った
/// 直後に滞留入力の破棄を開始する。`return` / `?` で早期退出しても必ず後で走る。
#[cfg(not(windows))]
// Some newer std versions renamed `fetch_update` to `try_update` and marked the
// old name deprecated; the not-windows code is the only place it compiles, so
// keep the old name (works on every toolchain) and silence the lint.
#[allow(deprecated)]
fn dropping_stale_input<T>(dialog: impl FnOnce() -> T) -> T {
	let result = dialog();
	if let Ok(mut deadline) = STALE_INPUT_DEADLINE.lock() {
		*deadline = Some(std::time::Instant::now() + STALE_INPUT_GRACE);
	}
	STALE_INPUT_TURNS.store(STALE_INPUT_TURNS_TO_DROP, std::sync::atomic::Ordering::Relaxed);
	for _ in 0..STALE_INPUT_TURNS_TO_DROP {
		let _ = slint::invoke_from_event_loop(|| {
			let _ = STALE_INPUT_TURNS.fetch_update(
				std::sync::atomic::Ordering::Relaxed,
				std::sync::atomic::Ordering::Relaxed,
				|n| Some(n.saturating_sub(1)),
			);
		});
	}
	result
}

// エラー / 情報のメッセージボックス。fire-and-forget で、子プロセスが終了する
// まで親ウィンドウの入力をブロックする (Windows は OS 側 grab、それ以外は下の
// DIALOG_ACTIVE)。
//
// The dialog lives in another process, so nothing stops this window from reacting
// to clicks while it is up. zenity/kdialog get `--modal`: on X11 GTK then takes an
// application-modal grab, which stops pointer/keyboard events for every other
// client -- including this window. On top of that we drop input events on our own
// window while the dialog is up, so the behaviour also holds on Wayland (no
// server-side grab) and under Xvfb (no WM).
#[cfg(target_os = "linux")]
pub fn show_message_box(title: &str, text: &str, kind: MessageIcon) {
	// 1 フレーム遅らせてから表示する。GTK ダイアログはネストしたループで
	// ブロックし、その間 slint ウィンドウは再描画されない。呼び出しと同ターン
	// で表示すると「メニューを閉じた直後の描画」が行われないまま、閉じたはず
	// のポップアップが残って見えるため。
	let title = title.to_owned();
	let text = text.to_owned();
	slint::Timer::single_shot(std::time::Duration::from_millis(16), move || {
		show_message_box_impl(&title, &text, kind);
	});
}

#[cfg(target_os = "linux")]
fn show_message_box_impl(title: &str, text: &str, kind: MessageIcon) {
	// 同一プロセスの GTK 標準ダイアログ (モーダルにブロックする)。
	if crate::gtk::gtk_ready() {
		use gtk::prelude::*;
		let dlg = gtk::MessageDialog::new(
			None::<&gtk::Window>,
			gtk::DialogFlags::empty(),
			if matches!(kind, MessageIcon::Error) {
				gtk::MessageType::Error
			} else {
				gtk::MessageType::Info
			},
			gtk::ButtonsType::Ok,
			text,
		);
		dlg.set_title(title);
		crate::gtk::run_dialog_blocking(&dlg);
		return;
	}
	// フォールバック: 別プロセスの zenity/kdialog/xmessage (fire-and-forget)。
	let title = title.to_owned();
	let text = text.to_owned();
	let (zflag, kflag) = match kind {
		MessageIcon::Error => ("--error", "error"),
		MessageIcon::Info => ("--info", "msgbox"),
	};
	let child = std::process::Command::new("zenity")
		.args([zflag, "--modal", "--title", &title, "--text", &text])
		.spawn()
		.or_else(|_| {
			std::process::Command::new("kdialog")
				.args(["--title", &title, kflag, &text])
				.spawn()
		})
		.or_else(|_| {
			std::process::Command::new("xmessage")
				.args(["-title", &title, &text])
				.spawn()
		});
	dialog_enter();
	match child {
		Ok(mut child) => {
			// Wait on a helper thread: Slint's event loop has to keep running
			// (it is what dispatches/keeps the window alive), but our input
			// stays blocked until the dialog is dismissed.
			std::thread::spawn(move || {
				let _ = child.wait();
				dialog_leave();
			});
		}
		Err(_) => {
			eprintln!("{title}: {text}");
			dialog_leave();
		}
	}
}

/// `display alert ... as critical / informational`。文字列は AppleScript ソース
/// に埋め込まず argv で渡すので、引用符やバックスラッシュ、変数の文字コードを
/// 壊さずにそのまま渡せる。
#[cfg(target_os = "macos")]
pub fn show_message_box(title: &str, text: &str, kind: MessageIcon) {
	let title = title.to_owned();
	let text = text.to_owned();
	let alert_kind = match kind {
		MessageIcon::Error => "critical",
		MessageIcon::Info => "informational",
	};
	let script = format!("display alert (item 1 of argv) message (item 2 of argv) as {alert_kind}");
	let child = std::process::Command::new("osascript")
		.args(["-e", "on run argv", "-e", script.as_str(), "-e", "end run", "--"])
		.arg(&title)
		.arg(&text)
		.spawn();
	dialog_enter();
	match child {
		Ok(mut child) => {
			std::thread::spawn(move || {
				let _ = child.wait();
				dialog_leave();
			});
		}
		Err(_) => {
			eprintln!("{title}: {text}");
			dialog_leave();
		}
	}
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
pub fn show_message_box(title: &str, text: &str, _kind: MessageIcon) {
	eprintln!("{title}: {text}");
}

/// メッセージボックスのボタン構成 (アイコンの `MessageIcon` と同じく定数で選ぶ)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageButtons {
	/// OK のみ。
	Ok,
	/// OK / キャンセル。
	OkCancel,
	/// はい / いいえ。
	YesNo,
	/// はい / いいえ / キャンセル。
	YesNoCancel,
}

/// `message_box` で押されたボタン。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MessageResponse {
	Ok,
	Cancel,
	Yes,
	No,
}

/// OS 標準のモーダルメッセージボックス。`buttons` でボタン構成を選び、押された
/// ボタンを返す。ラベルは各 OS の言語に任せる (カスタム文言が必要なら
/// `message_box_custom` を使う)。閉じられた / 表示できないときは `None`。
/// `show_message_box` (fire-and-forget) と違い回答までブロックする。
#[cfg(windows)]
pub fn message_box(title: &str, text: &str, buttons: MessageButtons) -> Option<MessageResponse> {
	use windows_sys::Win32::UI::Controls::{
		TDCBF_CANCEL_BUTTON, TDCBF_NO_BUTTON, TDCBF_OK_BUTTON, TDCBF_YES_BUTTON,
	};
	let (common, default, fallback) = match buttons {
		MessageButtons::Ok => (TDCBF_OK_BUTTON, IDOK, MessageResponse::Ok),
		MessageButtons::OkCancel => (
			TDCBF_OK_BUTTON | TDCBF_CANCEL_BUTTON,
			IDOK,
			MessageResponse::Cancel,
		),
		MessageButtons::YesNo => (
			TDCBF_YES_BUTTON | TDCBF_NO_BUTTON,
			IDYES,
			MessageResponse::No,
		),
		MessageButtons::YesNoCancel => (
			TDCBF_YES_BUTTON | TDCBF_NO_BUTTON | TDCBF_CANCEL_BUTTON,
			IDYES,
			MessageResponse::Cancel,
		),
	};
	let icon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_INFORMATION) };
	let rc = task_dialog_center_parent(title, text, icon, common, default);
	Some(match rc {
		IDOK => MessageResponse::Ok,
		IDYES => MessageResponse::Yes,
		IDNO => MessageResponse::No,
		IDCANCEL => MessageResponse::Cancel,
		_ => fallback,
	})
}

#[cfg(target_os = "linux")]
pub fn message_box(title: &str, text: &str, buttons: MessageButtons) -> Option<MessageResponse> {
	// 同一プロセスの GTK 標準ダイアログ。ボタンは GTK の標準 (OS 言語)。
	if crate::gtk::gtk_ready() {
		use gtk::prelude::*;
		let (btype, manual) = match buttons {
			MessageButtons::Ok => (gtk::ButtonsType::Ok, false),
			MessageButtons::OkCancel => (gtk::ButtonsType::OkCancel, false),
			MessageButtons::YesNo => (gtk::ButtonsType::YesNo, false),
			// GTK に Yes/No/Cancel の定型は無いので手動で足す。
			MessageButtons::YesNoCancel => (gtk::ButtonsType::None, true),
		};
		let dlg = gtk::MessageDialog::new(
			None::<&gtk::Window>,
			gtk::DialogFlags::empty(),
			gtk::MessageType::Question,
			btype,
			text,
		);
		dlg.set_title(title);
		if manual {
			dlg.add_button("gtk-yes", gtk::ResponseType::Yes);
			dlg.add_button("gtk-no", gtk::ResponseType::No);
			dlg.add_button("gtk-cancel", gtk::ResponseType::Cancel);
		}
		dlg.set_default_response(match buttons {
			MessageButtons::Ok | MessageButtons::OkCancel => gtk::ResponseType::Ok,
			MessageButtons::YesNo | MessageButtons::YesNoCancel => gtk::ResponseType::Yes,
		});
		let ret = crate::gtk::run_dialog_blocking(&dlg);
		return match ret {
			gtk::ResponseType::Ok => Some(MessageResponse::Ok),
			gtk::ResponseType::Yes => Some(MessageResponse::Yes),
			gtk::ResponseType::No => Some(MessageResponse::No),
			gtk::ResponseType::Cancel => Some(MessageResponse::Cancel),
			_ => None,
		};
	}
	// GTK が使えないときの保険: zenity → kdialog。
	dropping_stale_input(|| {
		let status = |cmd: &str, args: &[&str]| {
			std::process::Command::new(cmd).args(args).status().ok()
		};
		match buttons {
			MessageButtons::Ok => {
				if let Some(s) = status("zenity", &["--info", "--modal", "--title", title, "--text", text]) {
					return s.success().then_some(MessageResponse::Ok);
				}
				if let Some(s) = status("kdialog", &["--title", title, "--msgbox", text]) {
					return s.success().then_some(MessageResponse::Ok);
				}
			}
			MessageButtons::OkCancel | MessageButtons::YesNo => {
				let (ok_label, cancel_label, ok_resp, cancel_resp) = match buttons {
					MessageButtons::OkCancel => ("OK", "Cancel", MessageResponse::Ok, MessageResponse::Cancel),
					_ => ("Yes", "No", MessageResponse::Yes, MessageResponse::No),
				};
				if let Some(s) = status("zenity", &[
					"--question", "--modal", "--title", title, "--text", text,
					"--ok-label", ok_label, "--cancel-label", cancel_label,
				]) {
					return Some(if s.success() { ok_resp } else { cancel_resp });
				}
				if let Some(s) = status("kdialog", &["--title", title, "--yesno", "--modal", text]) {
					return Some(if s.success() { ok_resp } else { cancel_resp });
				}
			}
			MessageButtons::YesNoCancel => {
				if let Some(s) = status("kdialog", &["--title", title, "--yesnocancel", "--modal", text]) {
					return Some(match s.code() {
						Some(0) => MessageResponse::Yes,
						Some(1) => MessageResponse::No,
						_ => MessageResponse::Cancel,
					});
				}
				if let Some(s) = status("zenity", &[
					"--question", "--modal", "--title", title, "--text", text,
					"--ok-label", "Yes", "--cancel-label", "No",
				]) {
					return Some(if s.success() { MessageResponse::Yes } else { MessageResponse::No });
				}
			}
		}
		eprintln!("{title}: {text}");
		None
	})
}

// osascript に AppleScript を渡し、表示する文字列は argv で渡すヘルパ。
// `on run argv` + `--` 以降がスクリプトの argv になるため、タイトルや本文に
// 引用符・逆スラッシュ・改行・Unicode が含まれてもそのまま渡せる。
#[cfg(target_os = "macos")]
fn osascript(script: &[&str], argv: &[&str]) -> std::io::Result<std::process::Output> {
	std::process::Command::new("osascript")
		.args(script)
		.arg("--")
		.args(argv)
		.output()
}

#[cfg(target_os = "macos")]
pub fn message_box(title: &str, text: &str, buttons: MessageButtons) -> Option<MessageResponse> {
	dropping_stale_input(|| {
		// `display dialog` は押されたボタンのラベルを stdout に出す。ボタンは
		// 左から並び、既定は `default button` で指定する。Esc は終了コード 1。
		let (list, default) = match buttons {
			MessageButtons::Ok => ("\"OK\"", "OK"),
			MessageButtons::OkCancel => ("\"Cancel\", \"OK\"", "OK"),
			MessageButtons::YesNo => ("\"No\", \"Yes\"", "Yes"),
			MessageButtons::YesNoCancel => ("\"Cancel\", \"No\", \"Yes\"", "Yes"),
		};
		let display = format!(
			"display dialog (item 2 of argv) with title (item 1 of argv) buttons {{{list}}} default button \"{default}\" with icon caution"
		);
		let script = ["on run argv", display.as_str(), "end run"];
		match osascript(&script, &[title, text]) {
			Ok(out) if out.status.success() => {
				match String::from_utf8_lossy(&out.stdout).trim() {
					"OK" => Some(MessageResponse::Ok),
					"Yes" => Some(MessageResponse::Yes),
					"No" => Some(MessageResponse::No),
					"Cancel" => Some(MessageResponse::Cancel),
					_ => None,
				}
			}
			Ok(_) | Err(_) => {
				eprintln!("{title}: {text}");
				None
			}
		}
	})
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn message_box(_title: &str, _text: &str, _buttons: MessageButtons) -> Option<MessageResponse> {
	None
}

/// ボタンラベル中の `&X` をアクセラレータ (Alt+X) として取り出す。`&` を除いた
/// 表示文字列と、あればキー文字を返す。Windows の TaskDialog が `&` を
/// ニーモニックとして解釈するのと同じ表記を GTK でも再現するためのもの。
#[cfg(target_os = "linux")]
fn split_mnemonic(label: &str) -> (String, Option<char>) {
	match label.find('&') {
		Some(pos) => {
			let mut display = String::with_capacity(label.len());
			display.push_str(&label[..pos]);
			let mut chars = label[pos + 1..].chars();
			let accel = chars.next().map(|c| c.to_ascii_lowercase());
			display.extend(chars);
			(display, accel)
		}
		None => (label.to_owned(), None),
	}
}

/// ボタンラベルをカスタム指定できるモーダルメッセージボックス。押された
/// ボタンの `choices` 内の添字を返す。Esc / × で閉じられた場合は `None`。
/// OS 標準のボタン構成でよい場合は `message_box` を使う。
///
/// ボタンの意味づけはプラットフォームごとに次のとおり:
/// - Windows: `choices` の順にそのままボタンを並べ、`default` を既定ボタンにする。
/// - Linux: `choices[0]` を zenity の OK (Enter)、`choices[last]` をキャンセル/
///   閉じるボタン、それ以外を追加ボタンとして出す。kdialog はメニュー形式。
/// - macOS: リストから選ばせる。`default` を既定選択にする。
///
/// キャンセル (Esc / 閉じるボタン) は `None`。ラベル中の `&X` はアクセラレータ。
pub fn message_box_custom(
	title: &str,
	text: &str,
	choices: &[&str],
	default: Option<usize>,
) -> Option<usize> {
	if choices.is_empty() {
		return None;
	}
	#[cfg(windows)]
	{
		use windows_sys::Win32::UI::Controls::{
			TaskDialogIndirect, TASKDIALOG_BUTTON, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0,
			TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW, TDF_USE_HICON_MAIN,
		};
		use windows_sys::Win32::UI::Shell::{SHGetStockIconInfo, SHGSI_ICON, SHSTOCKICONINFO, SIID_HELP};
		use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_QUESTION};

		let title_w = encode_wide(title);
		let main_w = encode_wide(text);
		let mut buttons: Vec<TASKDIALOG_BUTTON> = Vec::new();
		// nButtonID に 100 + 添字 を振る (自身の識別にのみ使う)。
		let labels: Vec<Vec<u16>> = choices.iter().map(|c| encode_wide(c)).collect();
		let ids: Vec<i32> = (0..choices.len()).map(|i| 100 + i as i32).collect();
		for (label, id) in labels.iter().zip(ids.iter()) {
			buttons.push(TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: label.as_ptr() });
		}

		// 旧式の白抜き「?」(IDI_QUESTION) ではなく、Vista 以降のモダンな「?」
		// ストックアイコン (SIID_HELP) を SHGetStockIconInfo で取得して表示する。
		let mut icon_info = unsafe { std::mem::zeroed::<SHSTOCKICONINFO>() };
		icon_info.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
		let modern_question = unsafe { SHGetStockIconInfo(SIID_HELP, SHGSI_ICON, &mut icon_info) } >= 0;

		let mut config = TASKDIALOGCONFIG::default();
		config.cbSize = std::mem::size_of::<TASKDIALOGCONFIG>() as u32;
		config.hwndParent = DIALOG_OWNER.load(std::sync::atomic::Ordering::Relaxed);
		config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW | TDF_USE_HICON_MAIN;
		config.pszWindowTitle = title_w.as_ptr();
		config.Anonymous1 = TASKDIALOGCONFIG_0 {
			// hMainIcon を使うには TDF_USE_HICON_MAIN が必須: 無いと TaskDialog が
			// union を文字列ポインタとして扱い、アイコンハンドルを dereference して
			// アクセス違反で落ちる (0xC0000005)。
			hMainIcon: if modern_question {
				icon_info.hIcon
			} else {
				// ストックアイコン取得に失敗した場合のフォールバック。
				unsafe { LoadIconW(std::ptr::null_mut(), IDI_QUESTION) }
			},
		};
		// 本文は pszContent (黒字の detail スロット) へ入れる。
		config.pszMainInstruction = std::ptr::null();
		config.pszContent = main_w.as_ptr();
		config.cButtons = buttons.len() as u32;
		config.pButtons = buttons.as_ptr();
		config.nDefaultButton = default.map_or(ids[0], |i| ids[i.min(ids.len() - 1)]);

		let mut result = 0i32;
		let hr = unsafe { TaskDialogIndirect(&config, &mut result, std::ptr::null_mut(), std::ptr::null_mut()) };
		if hr != 0 {
			return None;
		}
		let id = result;
		(0..choices.len()).find(|&i| ids[i] == id)
	}
	#[cfg(target_os = "linux")]
	{
		// 同一プロセスの GTK 標準ダイアログ。ラベルの `&X` は Alt+X の
		// アクセラレータとして効かせる。
		if crate::gtk::gtk_ready() {
			use gtk::prelude::*;
			let dlg = gtk::MessageDialog::new(
				None::<&gtk::Window>,
				gtk::DialogFlags::empty(),
				gtk::MessageType::Question,
				gtk::ButtonsType::None,
				text,
			);
			dlg.set_title(title);
			let group = gtk::AccelGroup::new();
			dlg.add_accel_group(&group);
			for (i, label) in choices.iter().enumerate() {
				let (display, accel) = split_mnemonic(label);
				let resp = gtk::ResponseType::Other(100 + i as u16);
				let btn = dlg.add_button(&display, resp);
				if let Some(c) = accel {
					let keyval = gtk::gdk::keys::Key::from_name(&c.to_string());
					if *keyval != 0 {
						btn.add_accelerator(
							"clicked",
							&group,
							*keyval,
							gtk::gdk::ModifierType::MOD1_MASK,
							gtk::AccelFlags::VISIBLE,
						);
					}
				}
			}
			let default_resp = gtk::ResponseType::Other(
				100 + default.unwrap_or(0).min(choices.len() - 1) as u16,
			);
			dlg.set_default_response(default_resp);
			let ret = crate::gtk::run_dialog_blocking(&dlg);
			return match ret {
				gtk::ResponseType::Other(n) if n >= 100 && ((n - 100) as usize) < choices.len() => {
					Some((n - 100) as usize)
				}
				_ => None,
			};
		}
	}
	#[cfg(target_os = "linux")]
	return dropping_stale_input(|| {
		// `default` は Windows/macOS の実装だけが使う (zenity/kdialog は既定
		// ボタンを指定できる形で出さない)。参照しておかないと Linux ビルドで
		// 未使用引数の警告になる。
		let _ = &default;
		let zenity = {
			// first = OK (Enter), last = キャンセル, 中間 = 追加ボタン。
			let (first, mid, last) = match choices {
				[first, mid @ .., last] => (*first, mid, *last),
				[only] => (*only, &[][..], *only),
				[] => unreachable!(),
			};
			let mut args = vec![
				"--question", "--modal", "--title", title, "--text", text,
				"--ok-label", first, "--cancel-label", last,
			];
			for extra in mid {
				args.push("--extra-button");
				args.push(extra);
			}
			std::process::Command::new("zenity").args(&args).output()
		};
		if let Ok(out) = zenity {
			if out.status.success() { return Some(0); }
			// zenity の追加ボタンは exit 1 で「押したボタンのラベル」を stdout に出す。
			let combined = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
			for (i, label) in choices.iter().enumerate().skip(1) {
				if combined.contains(label) {
					return Some(i);
				}
			}
			return None;
		}
		let kdialog = {
			let mut args: Vec<String> = vec![
				"--title".into(), title.into(), "--menu".into(), text.into(),
			];
			for (i, label) in choices.iter().enumerate() {
				args.push(i.to_string());
				args.push(label.to_string());
			}
			std::process::Command::new("kdialog").args(&args).output()
		};
		if let Ok(out) = kdialog {
			if out.status.success() {
				let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
				return s.parse::<usize>().ok().and_then(|i| (i < choices.len()).then_some(i));
			}
		}
		eprintln!("{text}");
		None
	});
	#[cfg(target_os = "macos")]
	return dropping_stale_input(|| {
		// `choose from list` は選んだラベルを stdout に出す (Esc は終了コード 1)。
		// 選択肢も argv で渡すので、引用符を含むラベルを AppleScript のソースに
		// 埋め込む必要がない。
		const SCRIPT: [&str; 8] = [
			"on run argv",
			"set opts to {}",
			"repeat with i from 3 to (count of argv)",
			"set end of opts to item i of argv",
			"end repeat",
			"set chosen to choose from list opts with title (item 1 of argv) with prompt (item 2 of argv) default items {(item arith of argv)}",
			"return chosen as text",
			"end run",
		];
		let mut argv: Vec<&str> = Vec::with_capacity(3 + choices.len());
		argv.push(title);
		argv.push(text);
		argv.extend_from_slice(choices);
		// argv = [title, text, choices...]。choose from list の default items には
		// 「3 + default 添字」番目の argv 要素 (ラベル) を渡す。
		let default_one_based = 3 + default.unwrap_or(0).min(choices.len() - 1);
		let script = {
			let mut s: Vec<String> = Vec::new();
			for part in SCRIPT {
				s.push(part.replace("(item arith of argv)", &format!("(item {default_one_based} of argv)")));
			}
			s
		};
		let parts: Vec<&str> = script.iter().map(|s| s.as_str()).collect();
		if let Ok(out) = osascript(&parts, &argv) {
			if out.status.success() {
				let chosen = String::from_utf8_lossy(&out.stdout).trim().to_string();
				if let Some(i) = choices.iter().position(|c| *c == chosen) {
					return Some(i);
				}
			}
		}
		// ダイアログが使えないときは安全側: キャンセル扱い。
		eprintln!("{text}");
		None
	});
	#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
	{
		eprintln!("{title}: {text}");
		None
	}
}

// ---------- Folder picker ----------

#[cfg(windows)]
pub fn prompt_folder(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
	use windows::core::{HSTRING, IUnknown};
	use windows::Win32::Foundation::HWND as W32_HWND;
	use windows::Win32::System::Com::{
		CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx,
		CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
	};
	use windows::Win32::UI::Shell::{
		FileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, IFileOpenDialog,
		IShellItem, SHCreateItemFromParsingName, SIGDN_FILESYSPATH,
	};

	// 親ウィンドウは capture_dialog_owner が記録した HWND。OS はこれを owner
	// (モーダル) かつ配置の基準 (hwndOwner の中央) として使う。
	let owner_hwnd = DIALOG_OWNER.load(std::sync::atomic::Ordering::Relaxed);

	// IFileDialog は COM コンポーネントなので、使うスレッド上で初期化する。
	// S_OK ならここで初期化したので後で解放、S_FALSE (既に初期化済み) なら解放しない。
	let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
	let initialized_here = hr.is_ok();

	let picked = (|| {
		// FileOpenDialog (CLSID_FileOpenDialog) を IFileOpenDialog として生成。
		let dialog: IFileOpenDialog = unsafe {
			CoCreateInstance::<Option<&IUnknown>, IFileOpenDialog>(
				&FileOpenDialog,
				None,
				CLSCTX_INPROC_SERVER,
			)
		}
		.ok()?;
		// フォルダ選択専用モード。OS がダイアログを hwndOwner (=親ウィンドウ) の
		// 中央に配置する (SHBrowseForFolderW は画面中央固定なので使わない)。
		unsafe { dialog.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST) }
			.ok()?;
		let title = HSTRING::from(title.to_owned());
		unsafe { dialog.SetTitle(&title) }.ok()?;

		// 指定された initial フォルダへ最初から開く。無ければ OS 側の既定位置。
		let default_item: Option<IShellItem> = initial.and_then(|p| {
			let path = HSTRING::from(p.to_string_lossy().to_string());
			unsafe {
				SHCreateItemFromParsingName::<&HSTRING, Option<&IBindCtx>, IShellItem>(&path, None)
					.ok()
			}
		});
		unsafe { dialog.SetFolder(default_item.as_ref()) }.ok()?;

		unsafe { dialog.Show(Some(W32_HWND(owner_hwnd))) }.ok()?;
		let item = unsafe { dialog.GetResult() }.ok()?;
		let name = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?;
		// GetDisplayName が割り当てた文字列は CoTaskMemFree で解放する。
		let selected = unsafe { name.to_string() }.ok()?;
		unsafe { CoTaskMemFree(Some(name.as_ptr() as *const core::ffi::c_void)) };
		Some(PathBuf::from(selected))
	})();

	if initialized_here {
		unsafe { CoUninitialize() };
	}
	picked
}

#[cfg(target_os = "linux")]
pub fn prompt_folder(title: &str, initial: Option<&Path>) -> Option<PathBuf> {
	// 同一プロセスの GTK 標準 FileChooserDialog (モーダルにブロックする)。
	// ボタンは GTK の標準 (OS 言語) を stock id で出す。
	if crate::gtk::gtk_ready() {
		use gtk::prelude::*;
		let dlg = gtk::FileChooserDialog::with_buttons(
			Some(title),
			None::<&gtk::Window>,
			gtk::FileChooserAction::SelectFolder,
			&[
				("gtk-open", gtk::ResponseType::Accept),
				("gtk-cancel", gtk::ResponseType::Cancel),
			],
		);
		dlg.set_default_response(gtk::ResponseType::Accept);
		if let Some(p) = initial {
			let _ = dlg.set_current_folder(p);
		}
		let ret = crate::gtk::run_dialog_blocking(&dlg);
		let picked = if ret == gtk::ResponseType::Accept {
			dlg.filename()
		} else {
			None
		};
		return picked;
	}
	dropping_stale_input(|| {
		// GNOME (zenity) first, then KDE (kdialog); both print the picked folder on
		// stdout, so parse it. A missing helper falls through to the next one, but a
		// dialog the user actually cancelled (exit code 1) returns right away instead
		// of popping a second chooser on top.
		for cmd in [
			&["zenity", "--file-selection", "--directory", "--title", title][..],
			&["kdialog", "--getexistingdirectory", title][..],
		] {
			match std::process::Command::new(cmd[0]).args(&cmd[1..]).output() {
				Ok(out) if out.status.success() => {
					let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
					if !s.is_empty() {
						return Some(PathBuf::from(s));
					}
				}
				Ok(_) => return None,
				Err(_) => {}
			}
		}
		None
	})
}

#[cfg(target_os = "macos")]
pub fn prompt_folder(title: &str, _initial: Option<&Path>) -> Option<PathBuf> {
	dropping_stale_input(|| {
		// `choose folder` は alias を返すので、POSIX path にしてから出力させる。
		// キャンセル時は終了コード 1 なので None になる。
		const SCRIPT: [&str; 4] = [
			"on run argv",
			"set f to choose folder with prompt (item 1 of argv)",
			"return POSIX path of f",
			"end run",
		];
		let out = osascript(&SCRIPT, &[title]).ok()?;
		if !out.status.success() {
			return None;
		}
		let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
		if s.is_empty() {
			None
		} else {
			Some(PathBuf::from(s))
		}
	})
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn prompt_folder(_title: &str, _initial: Option<&Path>) -> Option<PathBuf> {
	None
}