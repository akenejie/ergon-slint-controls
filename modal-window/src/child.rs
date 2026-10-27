//! 子ウィンドウ (別 OS ウィンドウ) モーダルの制御。
//!
//! アプリは [`ModalChild`] を実装したダイアログを [`open_child_modal`] へ渡す
//! だけでよく、表示・モーダル化 (所有・中央配置・タイトルバー整え・親の入力
//! 遮断)・Esc/Enter/閉じるボタンの読み替え・破棄はこのモジュールが担う。

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::{backend, ModalChild, MODAL_MODE};

// --- Windows: タイトルバー整え・所有・中央配置・親の無効化 ---

// Windows: 子ウィンドウのタイトル バーを FormBorderStyle.FixedDialog 相当
// (閉じるボタンのみ・アイコン無し) にする。アイコン非表示はウィンドウ生成時
// から WS_EX_DLGMODALFRAME (ダイアログ枠) を持たせる方式で、patches/winit の
// with_dialog_modal_frame が WindowFlags として保持するため、winit のスタイル
// 再計算でも外れない。こちらでは残りの所望スタイル (閉じるボタンのみ・非リサイズ
// ・タスクバー非表示) を冪等に整えて維持する。
//
// 既に所望のスタイルなら何もしない (フレーム再描画によるちらつきを避ける)。
#[cfg(windows)]
pub fn enforce_modal_styles(child: &slint::Window) {
	use ergon_slint_controls::platform::main_window_hwnd;
	use windows_sys::Win32::Graphics::Gdi::{RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow};
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
		SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION,
		WS_EX_APPWINDOW, WS_EX_DLGMODALFRAME, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_THICKFRAME, WS_SYSMENU,
	};

	let child_hwnd = main_window_hwnd(child);
	if child_hwnd.is_null() {
		return;
	}

	unsafe {
		let current_style = GetWindowLongPtrW(child_hwnd, GWL_STYLE) as u32;
		let current_exstyle = GetWindowLongPtrW(child_hwnd, GWL_EXSTYLE) as u32;

		// 目的のスタイルを正しく構築
		let desired_style = (current_style | WS_CAPTION | WS_SYSMENU)
			& !(WS_MAXIMIZEBOX | WS_MINIMIZEBOX | WS_THICKFRAME);

		let desired_ex = (current_exstyle & !WS_EX_APPWINDOW) | WS_EX_DLGMODALFRAME;

		// 既に適用済みなら何もしない
		if current_style == desired_style && current_exstyle == desired_ex {
			return;
		}

		// 1. スタイルを一気に適用
		SetWindowLongPtrW(child_hwnd, GWL_STYLE, desired_style as isize);
		SetWindowLongPtrW(child_hwnd, GWL_EXSTYLE, desired_ex as isize);

		// 2. フレーム変更を確実にOSに通知して再描画
		SetWindowPos(
			child_hwnd,
			std::ptr::null_mut(),
			0, 0, 0, 0,
			SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
		);

		RedrawWindow(
			child_hwnd,
			std::ptr::null(),
			std::ptr::null_mut(),
			RDW_INVALIDATE | RDW_FRAME | RDW_UPDATENOW,
		);
	}
}

// タイトルバーのスタイル維持は Windows 専用。他プラットフォームでは何もしない。
#[cfg(not(windows))]
pub fn enforce_modal_styles(_child: &slint::Window) {}

// Windows: 子ウィンドウを親の所有ウィンドウにし、親の中央へ配置し、タイトル
// バーを FixedDialog 相当に整え (enforce_modal_styles)、親を無効化して
// モーダルにする。閉じるときは dismiss_modal_dialog を呼ぶこと。
#[cfg(windows)]
pub fn present_modal_dialog(parent: &slint::Window, child: &slint::Window) {
	use ergon_slint_controls::platform::main_window_hwnd;
	use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
	use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_HWNDPARENT};
	let parent_hwnd = main_window_hwnd(parent);
	let child_hwnd = main_window_hwnd(child);
	if child_hwnd.is_null() {
		return;
	}
	unsafe {
		// 所有者を親ウィンドウにする (親より手前に留まる)。
		SetWindowLongPtrW(child_hwnd, GWLP_HWNDPARENT, parent_hwnd as isize);
	}
	enforce_modal_styles(child);
	// 位置 (CenterParent) は生成属性として backend フックが適用済み。
	// 生成後の SetWindowPos 移動はちらつくため行わない。
	// モーダル: 親ウィンドウを無効化して操作できないようにする。
	unsafe {
		EnableWindow(parent_hwnd, 0);
	}
}

// present_modal_dialog で無効化した親ウィンドウを再び有効化する。
#[cfg(windows)]
pub fn dismiss_modal_dialog(parent: &slint::Window) {
	use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
	use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
	let parent_hwnd = ergon_slint_controls::platform::main_window_hwnd(parent);
	if parent_hwnd.is_null() {
		return;
	}
	unsafe {
		EnableWindow(parent_hwnd, 1);
		SetForegroundWindow(parent_hwnd);
	}
}

// --- Windows 以外: WM へのモーダル ヒント + 親入力の遮断 ---

// Windows 以外はウィンドウ マネージャへ標準のモーダル ヒント (transient /
// modal / ダイアログ型 / 固定サイズ) を伝え、あわせて親ウィンドウの入力を
// ergon の dialogs の仕組みで落とす。winit 0.30 上流には transient/modal の
// API が無いため、winit パッチ フォーク (slint-modal-winit) に追加した
// set_transient_for / set_modal を使う (フォークは本クレートの必須要件)。
#[cfg(not(windows))]
pub fn present_modal_dialog(parent: &slint::Window, child: &slint::Window) {
	use slint::winit_030::winit::window::WindowButtons;
	use slint::winit_030::WinitWindowAccessor;
	// 固定サイズ + 閉じるボタンだけ、をウィンドウ マネージャへ要求する。
	// X11 の set_resizable(false) は MWM ヒント (最大化不可・min=max) を設定する。
	//
	// モーダル (親より前面・親を操作不可) は X11 の標準ヒントで伝える:
	// WM_TRANSIENT_FOR (ICCCM) で親ウィンドウを所有者として指定し、
	// _NET_WM_STATE_MODAL (EWMH) を付ける。どちらも対応したウィンドウ
	// マネージャが「暗黙のモーダル」として扱う。加えて親の入力を ergon の
	// dialogs で落とす (WM がヒントを無視する環境の保険)。
	// transient / modal ヒントは winit パッチ フォーク由来の API を使う
	// (フォーク無しのビルドは compile_error で排除済み)。
	#[cfg(all(feature = "winit-modal-hints", unix, not(target_os = "macos")))]
	let parent_xid = parent.with_winit_window(|pw| pw.xlib_window()).flatten();
	child.with_winit_window(|cw| {
		cw.set_resizable(false);
		cw.set_enabled_buttons(WindowButtons::CLOSE);
		#[cfg(all(feature = "winit-modal-hints", unix, not(target_os = "macos")))]
		{
			if let Some(xid) = parent_xid {
				cw.set_transient_for(Some(xid));
				cw.set_modal(true);
			}
		}
	});
	// 位置は座標指定せずウィンドウ マネージャに任せる。transient_for 付きの
	// ウィンドウを親基準に配置するのは Linux の流儀 (多くのディストリビューション
	// の WM が自動で親の中央/近傍に置く)。生成後の座標指定移動は、WM が決めた
	// 配置を上書きする OS のルール逸脱で、Ubuntu 実機ではちらつきとして観測
	// された。
	ergon_slint_controls::dialogs::dialog_push();
}

#[cfg(not(windows))]
pub fn dismiss_modal_dialog(_parent: &slint::Window) {
	ergon_slint_controls::dialogs::dialog_pop();
}

// --- セッション管理: 開く / 閉じる / モーダル化の後追い ---

/// 開いている子ダイアログのセッション。アプリは [`ModalSlot`] 経由で間接的に
/// 扱うだけでよく、中身はライブラリが管理する。
pub struct ModalSession {
	child: Rc<dyn ModalChild>,
	// (Windows) winit のスタイル再計算で閉じるボタン以外のボックスやリサイズ枠
	// が戻ってくるのを冪等に整え直す、開いている間だけ動くタイマー。
	// 破棄 (drop) すると停止する。
	#[allow(dead_code)]
	style_timer: Option<slint::Timer>,
}

/// 子ダイアログのスロット。閉じたら `None` に戻る (hide ではなく破棄)。
pub type ModalSlot = Rc<RefCell<Option<ModalSession>>>;
/// [`ModalSlot`] の弱い参照。タイマー コールバックからスロットへ触るのに使う。
pub type WeakModalSlot = std::rc::Weak<RefCell<Option<ModalSession>>>;

/// 子ダイアログを表示し、モーダル子ウィンドウとして整える。
///
/// 位置は OS に任せる: Windows は生成時 CenterParent (backend フック)、
/// Linux は WM の transient 配置 (多くのディストリビューションが親基準で
/// 配置する)。生成後の座標移動は OS のルール逸脱 (ちらつき) なので行わない。
/// タイトルバーのテーマは既定で親ウィンドウに揃える。閉じるときは必ず
/// [`close_child_modal`] を使う (hide ではなく破棄される)。
pub fn open_child_modal<P: ComponentHandle + 'static, C: ModalChild>(
	slot: &ModalSlot,
	parent: &P,
	child: Rc<C>,
) {
	if !MODAL_MODE.is_child_window() {
		return;
	}
	// 二重オープン ガード (既に開いているときは何もしない)。
	if slot.borrow().is_some() {
		return;
	}
	// 子ダイアログのタイトルバーは既定で親ウィンドウに揃える (OS のダイアログ
	// の慣習)。親のテーマは「アプリが明示指定したテーマ」を優先し (winit の
	// Window::theme() は X11 では XSETTINGS 依存で None を返すことがある)、
	// 未指定なら winit の現在テーマ、それも無ければ OS 既定に任せる。
	{
		use slint::winit_030::winit::window::Theme;
		use slint::winit_030::WinitWindowAccessor;
		let theme = match ergon_slint_controls::platform::applied_titlebar_theme() {
			Some(ergon_slint_controls::platform::TitlebarTheme::Light) => Some(Theme::Light),
			Some(ergon_slint_controls::platform::TitlebarTheme::Dark) => Some(Theme::Dark),
			_ => parent.window().with_winit_window(|pw| pw.theme()).flatten(),
		};
		backend::set_next_child_theme(theme);
	}
	let session = ModalSession {
		child: child.clone(),
		style_timer: start_style_timer(slot),
	};
	*slot.borrow_mut() = Some(session);
	// Esc = キャンセル / Enter = 確定 / 閉じるボタン = キャンセル を読み替える。
	install_event_hook(slot);
	let _ = child.show();
	// モーダル化はネイティブ ウィンドウが生成されてから適用する。
	schedule_apply_modal(parent.as_weak(), Rc::downgrade(slot));
}

/// 子ダイアログを閉じる (モーダル解除 + 破棄)。
///
/// 親の再有効化は即時に行い、子ウィンドウの破棄はコールバックの実行中に自
/// ウィンドウを壊さないよう、イベント ループへ戻ってから行う。
pub fn close_child_modal(slot: &ModalSlot, parent: &slint::Window) {
	// 既に閉じているときは何もしない (モーダル解除を二重に適用しない)。
	if slot.borrow().is_none() {
		return;
	}
	// 親を再有効化してモーダルを解除する。
	dismiss_modal_dialog(parent);
	let slot = slot.clone();
	slint::Timer::single_shot(std::time::Duration::ZERO, move || {
		let Some(session) = slot.borrow_mut().take() else { return };
		// Slint は表示中のトップレベル ウィンドウを保持し続けるため、hide()
		// してからハンドルを drop しないとネイティブ ウィンドウが破棄されない
		// (hide だけ・drop だけでは残る)。
		let _ = session.child.hide();
		// session (style_timer / child) を drop → 子コンポーネントの破棄。
	});
}

// (Windows) 開いている間だけスタイルを冪等に整え直すタイマーを開始する。
#[cfg(windows)]
fn start_style_timer(slot: &ModalSlot) -> Option<slint::Timer> {
	let weak = Rc::downgrade(slot);
	let timer = slint::Timer::default();
	timer.start(
		slint::TimerMode::Repeated,
		std::time::Duration::from_millis(100),
		move || {
			if let Some(slot) = weak.upgrade() {
				let guard = slot.borrow();
				if let Some(session) = guard.as_ref() {
					enforce_modal_styles(session.child.window());
				}
			}
		},
	);
	Some(timer)
}

#[cfg(not(windows))]
fn start_style_timer(_slot: &ModalSlot) -> Option<slint::Timer> {
	None
}

// 子ウィンドウの winit イベント フック。閉じるボタン (CloseRequested) は
// Slint が既定では隠すだけなので、破棄を伴う cancel 経路へ寄せるため
// PreventDefault で飲み込む。
fn install_event_hook(slot: &ModalSlot) {
	use slint::winit_030::WinitWindowAccessor;
	let weak = Rc::downgrade(slot);
	let guard = slot.borrow();
	let Some(session) = guard.as_ref() else { return };
	let window = session.child.window();
	// (X11/macOS) slint バックエンドは生成直後にホストのカラースキームを新規
	// ウィンドウへ流し込み (winitwindowadapter の create_window)、さらに XDG
	// 設定のポータル監視が全ウィンドウへ push する。テーマは生成後しばらく
	// イベントごとに再主張して勝たせる。テーマ指定はプロパティ書き換えのみで
	// ウィンドウ移動のような可視ちらつきはなく、落ち着いた後は OS 標準機構に
	// 任せる。
	let theme_until = std::cell::Cell::new(
		std::time::Instant::now() + std::time::Duration::from_millis(300),
	);
	window.on_winit_window_event(move |_win, event| {
		use slint::winit_030::EventResult;
		use slint::winit_030::winit::event::WindowEvent;
		let Some(slot) = weak.upgrade() else { return EventResult::Propagate };
		let child = slot.borrow().as_ref().map(|s| s.child.clone());
		let Some(child) = child else { return EventResult::Propagate };
		#[cfg(not(windows))]
		if std::time::Instant::now() < theme_until.get() {
			if let Some(theme) = backend::peek_next_child_theme() {
				let cw = child.window();
				cw.with_winit_window(|w| w.set_theme(Some(theme)));
			}
		}
		// (Windows) WS_EX_DLGMODALFRAME は winit パッチが生成時から維持する。
		// 残る所望スタイル (閉じるボタンのみ・非リサイズ) は winit が
		// set_window_flags のたびに再計算するため、イベントごとに冪等に
		// 整え直して維持する。
		#[cfg(windows)]
		enforce_modal_styles(child.window());
		match event {
			WindowEvent::CloseRequested => {
				child.cancel();
				EventResult::PreventDefault
			}
			WindowEvent::KeyboardInput { event, .. } => {
				if event.state.is_pressed() && !event.repeat {
					if let slint::winit_030::winit::keyboard::PhysicalKey::Code(code) =
						event.physical_key
					{
						use slint::winit_030::winit::keyboard::KeyCode;
						match code {
							KeyCode::Escape => {
								child.cancel();
								return EventResult::PreventDefault;
							}
							KeyCode::Enter | KeyCode::NumpadEnter => {
								child.accept();
								return EventResult::PreventDefault;
							}
							_ => {}
						}
					}
				}
				EventResult::Propagate
			}
			_ => EventResult::Propagate,
		}
	});
}

// show() 直後は Slint がネイティブ ウィンドウをまだ生成していないことがある
// ため、生成されてから中央モーダル化を適用する。
fn schedule_apply_modal<P: ComponentHandle + 'static>(
	parent_weak: slint::Weak<P>,
	slot_weak: WeakModalSlot,
) {
	slint::Timer::single_shot(std::time::Duration::ZERO, move || {
		apply_modal_when_ready(&parent_weak, &slot_weak, 0);
	});
}

fn apply_modal_when_ready<P: ComponentHandle + 'static>(
	parent_weak: &slint::Weak<P>,
	slot_weak: &WeakModalSlot,
	tries: u32,
) {
	let Some(slot) = slot_weak.upgrade() else { return };
	// ネイティブ ウィンドウが生成済みか。Windows は HWND、それ以外は
	// winit ウィンドウの有無で判定する。
	let ready = {
		let guard = slot.borrow();
		let Some(session) = guard.as_ref() else { return };
		let child_window = session.child.window();
		#[cfg(windows)]
		{
			!ergon_slint_controls::platform::main_window_hwnd(child_window).is_null()
		}
		#[cfg(not(windows))]
		{
			use slint::winit_030::WinitWindowAccessor;
			child_window.has_winit_window()
		}
	};
	if !ready {
		if tries < 200 {
			let parent_weak = parent_weak.clone();
			let slot_weak = slot_weak.clone();
			slint::Timer::single_shot(std::time::Duration::from_millis(5), move || {
				apply_modal_when_ready(&parent_weak, &slot_weak, tries + 1);
			});
		}
		return;
	}
	let Some(parent) = parent_weak.upgrade() else { return };
	{
		let guard = slot.borrow();
		let Some(session) = guard.as_ref() else { return };
		present_modal_dialog(parent.window(), session.child.window());
	}
	// 位置は OS に任せる (Windows: 生成時 CenterParent / Linux: WM の transient
	// 配置)。表示後の再配置は WM の決定を上書きする OS のルール逸脱であり、
	// Ubuntu 実機では「中央配置 → 座標移動」のちらつきとして観測されたため
	// 行わない。
}
