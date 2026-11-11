//! 子ウィンドウ (別 OS ウィンドウ) モーダルの制御。
//!
//! アプリは [`ModalChild`] を実装したダイアログを [`open_child_modal`] へ渡す
//! だけでよく、表示・モーダル化 (所有・中央配置・タイトルバー整え・親の入力
//! 遮断)・Esc/Enter/閉じるボタンの読み替え・破棄はこのモジュールが担う。

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use super::{backend, ModalChild};

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
	use crate::platform::window_hwnd;
	use windows_sys::Win32::Graphics::Gdi::{RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow};
	use windows_sys::Win32::UI::WindowsAndMessaging::{
		GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE,
		SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION,
		WS_EX_APPWINDOW, WS_EX_DLGMODALFRAME, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_THICKFRAME, WS_SYSMENU,
	};

	let child_hwnd = window_hwnd(child);
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
	use crate::platform::window_hwnd;
	use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
	use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_HWNDPARENT};
	let parent_hwnd = window_hwnd(parent);
	let child_hwnd = window_hwnd(child);
	if child_hwnd.is_null() {
		return;
	}
	unsafe {
		// 所有者を親ウィンドウにする (親より手前に留まる)。
		SetWindowLongPtrW(child_hwnd, GWLP_HWNDPARENT, parent_hwnd as isize);
	}
	enforce_modal_styles(child);
	// 位置 (所有者の中央) は winit パッチ フォークが「表示直前 (ShowWindow の
	// 直前)」に配置する — ライブラリは backend フックで
	// with_dialog_center_over_owner を指示するだけ。表示後の SetWindowPos 移動
	// はちらつくため行わない。
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
	let parent_hwnd = crate::platform::window_hwnd(parent);
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
	// (fork は modal-dialog feature が要求する)。
	#[cfg(all(unix, not(target_os = "macos")))]
	let _parent_xid = parent
		.with_winit_window(crate::platform::winit_xlib_window_id)
		.flatten();
	child.with_winit_window(|cw| {
	// X11 (linux): transient / modal / 固定サイズ / 閉じるボタンのみは**生成時
	// に焼き込み済み** (backend.rs の生成フック → winit フォーク modal.6+)。
	// マップ後にこれらを再送すると WM が状態変化として処理し、表示中の窓の再
	// 装飾 / 再配置 (= ちらつき) の引き金になるため何もしない。
	#[cfg(not(all(unix, not(target_os = "macos"))))]
	{
		use slint::winit_030::winit::window::WindowButtons;
		cw.set_resizable(false);
		cw.set_enabled_buttons(WindowButtons::CLOSE);
	}
	#[cfg(all(unix, not(target_os = "macos")))]
	{
		// X11 (Linux / Ubuntu の Mutter で実機調査して確定した設計。v0.3.53〜
		// v0.3.73 の一連の実験の結論):
		//
		// 1. **WM は窓の管理開始時 (最初のマップ処理) に transient / modal /
		//    user time / 型 / スタイルを一度だけ読む。** 生成後の変更は間に合
		//    わないことがある (slint の show() による遅延マップが post-creation
		//    の書き込みに先勝ちするため)。→ **すべて生成属性に焼き込む**
		//    (backend.rs の生成フック → winit フォーク modal.6+ の
		//    with_x11_transient_for / with_x11_modal / with_resizable /
		//    with_enabled_buttons / with_visible(false))。
		//    - transient は WM_TRANSIENT_FOR。Mutter は管理開始時にしか解決せ
		//      ず、親が管理表に無い / 解決できないと**親基準の配置をやめて画面
		//      中央に置く** (place.c は transient 解決時のみ親中央、それ以外は
		//      workarea 中央)。
		//    - modal は _NET_WM_STATE_MODAL。EWMH の ClientMessage はマップ前 /
		//      未管理の窓には WM に無視されるため、生成時のプロパティが必要。
		//    - user time は _NET_WM_USER_TIME。無いと「ユーザー操作と無関係に
		//      開いた窓」としてフォーカスを与えられない。
		// 2. **マップ後のヒント再送はしない。** WM が表示中の窓の状態変化とし
		//    て処理し、再装飾 / 再配置 = ちらつきの原因になる (実機で確認)。
		//    表示位置は WM の責任であり、クライアントが位置を指定・移動して
		//    もならない。
		// 3. **マップ後に行うのは user time の補完と活性化要求だけ** (下記)。
		//
		// Linux: 子ダイアログへユーザー操作の時刻を補完する。生成時に接続の最
		// 終入力時刻が焼き込まれる (winit フォーク modal.9) が、プログラム的
		// に開かれた場合など焼き込みが空だったときの保険。時刻の取得は
		// PropertyNotify トリックで行う (X プロトコルには現在時刻を問い合わせ
		// る API が無い)。親窓の時刻は winit フォーク (modal.5) が入力イベン
		// トのたびに自動更新するため、ここでは触らない。
		#[cfg(target_os = "linux")]
		{
			if let Some(child) = crate::platform::winit_xlib_window_id(cw) {
				let child = child as u32;
				let t = crate::gtk::current_server_time(child);
				if let Some(t) = t {
					crate::gtk::write_user_time(child, t);
				}
				// フォーカスされるまで活性化を続ける (Mutter は管理時に自分で
				// フォーカスを与えないため、マップ後の活性化要求が 1 回必要。
				// 詳細は gtk::activate_window_until_focused のドキュメント)。
				crate::gtk::activate_window_until_focused(child);
			}
		}
		// 非表示で生成した子ダイアログをここでマップする。
		cw.set_visible(true);
		// (診断コード) マップ / アンマップの遷移追跡。コメントアウト済み —
		// 再調査は src/gtk.rs の debug_trace_map_state を復活させる。
		// #[cfg(target_os = "linux")]
		// crate::gtk::debug_trace_map_state(crate::platform::winit_xlib_window_id(cw).map(|x| x as u32));
	}
	});
	crate::dialogs::dialog_push(parent);
}

#[cfg(not(windows))]
pub fn dismiss_modal_dialog(_parent: &slint::Window) {
	crate::dialogs::dialog_pop();
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
/// 子ウィンドウ方式の API。オーバーレイ方式のアプリから呼ばないこと
/// (方式の選択はアプリ側の [`ModalMode`] 定数で行う)。
///
/// 位置の決定責任は OS の文化による: Windows は OS が位置を決めないため
/// winit パッチ フォークが「表示直前」に所有者の中央へ配置し (ライブラリは
/// 生成時属性で指示)、Linux は WM が transient を親基準に配置する文化なので
/// ライブラリは座標を扱わない。いずれも表示後の座標移動は行わない
/// (ちらつきの原因)。タイトルバーのテーマは既定で親ウィンドウに揃える。
/// 閉じるときは必ず [`close_child_modal`] を使う (hide ではなく破棄される)。
pub fn open_child_modal<P: ComponentHandle + 'static, C: ModalChild>(
	slot: &ModalSlot,
	parent: &P,
	child: Rc<C>,
) {
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
		let theme = match crate::platform::applied_titlebar_theme() {
			Some(crate::platform::TitlebarTheme::Light) => Some(Theme::Light),
			Some(crate::platform::TitlebarTheme::Dark) => Some(Theme::Dark),
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
	window.on_winit_window_event(move |_win, event| {
		use slint::winit_030::EventResult;
		use slint::winit_030::winit::event::WindowEvent;
		let Some(slot) = weak.upgrade() else { return EventResult::Propagate };
		let child = slot.borrow().as_ref().map(|s| s.child.clone());
		let Some(child) = child else { return EventResult::Propagate };
	// (X11/macOS) slint バックエンドは生成直後にホストのカラースキームを新
	// 規ウィンドウへ流し込み (winitwindowadapter の create_window)、さらに
	// XDG 設定のポータル監視が全ウィンドウへ push する。生成時に焼き込んだ
	// テーマやサイズ固定が上書きされることがあり、タイトルバーが黒くなった
	// りリサイズできたりする (実測)。
	//
	// 再主張は**マップ前 (is_visible == false) に限定**する。マップ後の修正
	// は WM の再装飾 = 表示位置のガタつきになるため (実測)、見えないうちに
	// 済ませる。マップ後は正しい状態で表示されている前提とし、触らない。
	// 正しい状態での再主張は何もしない (X の同値プロパティ書き込みはイベント
	// を発生させないため、負荷もない)。テーマ指定はプロパティ書き換えのみで
	// ウィンドウ移動のような可視ちらつきはなく、落ち着いた後は OS 標準機構に
	// 任せる。
	#[cfg(not(windows))]
	{
		let cw = child.window();
		cw.with_winit_window(|w| {
			// None (未確定) は未マップ扱いにする。
			if w.is_visible() != Some(false) {
				return;
			}
			// 生成属性 (非リサイズ) が反映されなかった場合の再主張。
			// winit の is_resizable() はキャッシュしたヒントを読むだけ。
			//
			// 注意: winit の is_visible() は内部状態が YesWait (マップ要求済
			// み・マップ未確定) のときも Some(false) を返す。つまりこの再主張
			// はマップ要求後のイベントでも発火しうる。マップ後の
			// set_resizable(false) は WM_NORMAL_HINTS / _MOTIF_WM_HINTS の書
			// き換えになり、Mutter が装飾を作り直してクライアントがフレーム
			// 内で動く (表示直後の位置ずれ。実測: ConfigureNotify pos=(10,40)
			// → pos=(1,33))。v0.30.13-modal.14 のフォークで modal ウィンドウ
			// は生成時にヒントが凍結されるため、この呼び出しは安全な no-op
			// になった。フォークが無い (凍結の無い) winit では、ここでマップ
			// 後に set_resizable を呼ぶと位置ずれが再発する点に注意。
			if w.is_resizable() {
				w.set_resizable(false);
			}
			// テーマ: 生成時に焼き込んだ値 (親に揃える) を再主張する。
			if let Some(theme) = backend::peek_next_child_theme() {
				w.set_theme(Some(theme));
			}
		});
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
						match crate::modal_dialog::modal_key_action(code) {
							Some(crate::modal_dialog::ModalKeyAction::Cancel) => {
								child.cancel();
								return EventResult::PreventDefault;
							}
							Some(crate::modal_dialog::ModalKeyAction::Accept) => {
								child.accept();
								return EventResult::PreventDefault;
							}
							None => {}
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
			!crate::platform::window_hwnd(child_window).is_null()
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
	// 属性の検証 / 修正はイベント フック (install_event_hook) が**マップ前**
	// (is_visible == false の間) に冪等に行う。ここ (show() 後に走る) で
	// set_resizable / set_theme すると WM の再装飾 = 表示位置のガタつき /
	// ちらつきになるため触らない (実測)。
	let Some(parent) = parent_weak.upgrade() else { return };
	{
		let guard = slot.borrow();
		let Some(session) = guard.as_ref() else { return };
		present_modal_dialog(parent.window(), session.child.window());
	}
	// 位置の決定は Windows ではライブラリの責任 (生成時 with_position、
	// backend.rs)、Linux では WM の責任 (transient 配置)。表示後の再配置は
	// いずれの OS でもちらつきの原因になるため行わない (Ubuntu 実機では
	// 「WM 中央配置 → 座標移動」のちらつきとして観測された)。
}
