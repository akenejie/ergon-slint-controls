//! Linux の同一プロセス内 GTK ダイアログを「モーダルにブロック実行」するための
//! 汎用機構。ダイアログの中身 (タイトル・本文・ボタン) は呼び出し側が組み、
//! ここは「どうしたら winit の親窓に対してモーダルになるか」だけを面倒みる。
//!
//! 背景: Linux のダイアログを zenity/kdialog (別プロセス) に丸投げすると、
//! ウィンドウ・フォント・ボタンデザインが GTK アプリの標準とは別物になり、
//! VSCode (Electron) がやる「同一プロセス内で出し、モーダルにする」挙動とも
//! 違う。そこで GTK (libgtk-3) の標準ウィジェットをそのまま同一プロセス内に
//! 作り、`gtk::Dialog::run()` が回す GTK の再帰メインループでブロックする。
//!
//! モーダル化: GTK の `gtk_window_set_modal` は「同じアプリ (窓グループ)」の窓
//! しか止められず、親の winit 窓 (別 X 接続の別アプリ扱い) には効かない。
//!
//! 配置 + モーダル: GTK に transient parent を渡せない (親が GtkWindow でない)
//! ため、マップ前にダイアログの X 窓 (GDK から直接取得) へ `WM_TRANSIENT_FOR`
//! を直接設定する。親窓は `_NET_WM_PID` の付いた自分のトップレベル窓から選ぶ。
//! これで WM が親の中央に置き、`_NET_WM_STATE_MODAL` (set_modal 由来) と合わ
//! せて親をフォーカス/操作できないモーダルなペアとして扱う。
//!
//! 応答なし対策: winit のイベントループは GTK の run() 中は回らないため、WM が
//! 親窓へ送る `_NET_WM_PING` に応答できず「応答なし」と表示されうる。親窓の
//! `WM_PROTOCOLS` から一時的に外す。x11rb は書き込みをクライアント側に
//! バッファするので、必ず flush してから GTK ループに入る。
//!
//! 親への入力操作は上記の WM 側のモーダルで塞がるが、念のため GDK のシート
//! グラブも張る (best-effort)。

use std::sync::atomic::{AtomicBool, Ordering};

use gtk::prelude::*;

/// GTK を (プロセスで) 最初に一度だけ初期化する。失敗してもパニックせず
/// false を返し、呼び出し側が zenity 等へフォールバックできるようにする。
pub fn gtk_ready() -> bool {
	static READY: AtomicBool = AtomicBool::new(false);
	if READY.load(Ordering::Acquire) {
		return true;
	}
	if gtk::init().is_ok() {
		READY.store(true, Ordering::Release);
		true
	} else {
		false
	}
}

fn intern(conn: &x11rb::rust_connection::RustConnection, name: &[u8]) -> Option<u32> {
	use x11rb::protocol::xproto::ConnectionExt;
	conn.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
}

/// ダイアログを realize (X 窓生成、まだマップしない) し、WM に管理される
/// トップレベルの X 窓 id を返す。XID は GDK から直接取る。
///
/// かつては root 直下の子窓の差分 (realize 前後) で検出していたが、 realize と
/// マップの間で WM が reparent を先に済ませると窓が root 直下から消え、検出
/// できずに transient を付け損ねる。ダイアログを繰り返し表示すると WM 側の
/// 処理が速くなり、3 回目あたりから「子ウィンドウではなくモードレスになる」
/// 不具合の原因になった。GDK からの直接取得はこの競合がない。
fn realize_dialog_xids(
	dlg: &gtk::Dialog,
	_conn: &x11rb::rust_connection::RustConnection,
) -> Vec<u32> {
	dlg.realize();
	dlg.window()
		.and_then(|w| w.downcast_ref::<gdkx11::X11Window>().map(|x| x.xid() as u32))
		.into_iter()
		.collect()
}

/// ダイアログ窓が実際にマップされ Viewable になるまで (WM の reparent/マップを
/// 含めて) GTK のイベントを回して待つ。Viewable でないと gdk_seat_grab は
/// NotViewable で失敗する。
fn wait_viewable(dlg: &gtk::Window, timeout_ms: u128) -> bool {
	let start = std::time::Instant::now();
	loop {
		if dlg.window().is_some_and(|win| win.is_viewable()) {
			return true;
		}
		gtk::main_iteration_do(false);
		if start.elapsed().as_millis() > timeout_ms {
			return false;
		}
		std::thread::sleep(std::time::Duration::from_millis(2));
	}
}

/// ダイアログの X 窓を親 (winit メイン窓) の transient にする。WM は transient
/// なダイアログを親の中央に置き、親より下に降ろせなくする。
fn set_transient_for(conn: &x11rb::rust_connection::RustConnection, dialog: u32, parent: u32) {
	use x11rb::protocol::xproto::{AtomEnum, PropMode};
	use x11rb::wrapper::ConnectionExt as _;
	if let Some(prop) = intern(conn, b"WM_TRANSIENT_FOR") {
		let _ = conn
			.change_property32(PropMode::REPLACE, dialog, prop, AtomEnum::WINDOW, &[parent])
			.map(|c| c.ignore_error());
	}
}

/// X ウィンドウをフォーカスされるまで活性化し続ける (`_NET_ACTIVE_WINDOW`)。
///
/// WM が管理を開始した直後の窓への活性化要求は、WM 側のタイミングで無視さ
/// れることがある (Mutter で実機確認)。マップ直後から 60ms 間隔で
/// `_NET_ACTIVE_WINDOW` を確認し、フォーカスされていなければ要求を送る。フ
/// ォーカスされたら、または窓が消えた (閉じられた) ら停止する。要求のタイム
/// スタンプは 0 (EWMH の「時刻指定なし」)。Mutter は 0 ならタイムスタンプに
/// よる拒否チェックをスキップして自身で現在時刻を取得してフォーカスする。
pub fn activate_window_until_focused(win: u32) {
	use x11rb::connection::Connection as _;
	use x11rb::protocol::xproto::AtomEnum;
	use x11rb::protocol::xproto::ConnectionExt as _;
	use x11rb::protocol::xproto::ClientMessageEvent;
	use x11rb::protocol::xproto::EventMask;
	let Ok((conn, _)) = x11rb::connect(None) else { return };
	let Some(active_atom) = intern(&conn, b"_NET_ACTIVE_WINDOW") else { return };
	let root = conn.setup().roots[0].root;
	let focused = move |conn: &x11rb::rust_connection::RustConnection| -> bool {
		conn.get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 8)
			.ok()
			.and_then(|c| c.reply().ok())
			.and_then(|r| r.value32().and_then(|mut it| it.next()))
			.map(|w| w == win)
			.unwrap_or(false)
	};
	if focused(&conn) {
		return;
	}
	let ticks = std::cell::Cell::new(0u32);
	let holder = std::rc::Rc::new(std::cell::RefCell::new(None::<slint::Timer>));
	let holder2 = holder.clone();
	let timer = slint::Timer::default();
	timer.start(
		slint::TimerMode::Repeated,
		std::time::Duration::from_millis(60),
		move || {
			let n = ticks.get() + 1;
			ticks.set(n);
			if n > 20 || focused(&conn) {
				if let Some(t) = holder2.borrow_mut().take() {
					t.stop();
				}
				return;
			}
			// 窓が消えていたら (ダイアログが閉じられたら) 諦める。
			if conn.get_window_attributes(win).map(|c| c.reply().is_err()).unwrap_or(true) {
				if let Some(t) = holder2.borrow_mut().take() {
					t.stop();
				}
				return;
			}
			let Some(state) = intern(&conn, b"_NET_ACTIVE_WINDOW") else { return };
			let event = ClientMessageEvent::new(32, win, state, [1u32, 0, 0, 0, 0]).to_owned();
			let _ = conn
				.send_event(
					false,
					root,
					EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
					event,
				)
				.map(|c| c.ignore_error());
			let _ = conn.flush();
		},
	);
	*holder.borrow_mut() = Some(timer);
}

/// X ウィンドウをアクティブにする (`_NET_ACTIVE_WINDOW` ClientMessage)。
///
/// ウィンドウが WM に管理された (マップされた) 後に呼ぶこと。マップ前に
/// 送ると WM に無視される。`time` には要求の根拠となるユーザー操作の時刻
/// (`_NET_WM_USER_TIME` に使ったものと同じ値) を渡す。0 を渡すと WM のフォー
/// カス盗み取り防止に「時刻の分からない古い要求」と扱われ、Mutter は拒否す
/// るため、実時刻を渡すこと。
pub fn activate_window(win: u32, time: u32) {
	use x11rb::connection::Connection as _;
	use x11rb::protocol::xproto::ClientMessageEvent;
	use x11rb::protocol::xproto::ConnectionExt as _;
	use x11rb::protocol::xproto::EventMask;
	let Ok((conn, _)) = x11rb::connect(None) else { return };
	let Some(state) = intern(&conn, b"_NET_ACTIVE_WINDOW") else { return };
	let root = conn.setup().roots[0].root;
	let event = ClientMessageEvent::new(32, win, state, [1u32, time, 0, 0, 0]).to_owned();
	let _ = conn
		.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)
		.map(|c| c.ignore_error());
	let _ = conn.flush();
}

/// X ウィンドウへ `_NET_WM_USER_TIME` を書く。
pub fn write_user_time(win: u32, time: u32) {
	use x11rb::connection::Connection as _;
	use x11rb::protocol::xproto::AtomEnum;
	use x11rb::wrapper::ConnectionExt as _;
	let dbg = std::env::var("ERGON_MODAL_DEBUG").is_ok();
	let Ok((conn, _)) = x11rb::connect(None) else {
		if dbg {
			eprintln!("[modal-dbg] write_user_time({win}): 接続失敗");
		}
		return;
	};
	let Some(p) = intern(&conn, b"_NET_WM_USER_TIME") else {
		if dbg {
			eprintln!("[modal-dbg] write_user_time({win}): atom 取得失敗");
		}
		return;
	};
	match conn.change_property32(
		x11rb::protocol::xproto::PropMode::REPLACE,
		win,
		p,
		AtomEnum::CARDINAL,
		&[time],
	) {
		Ok(c) => {
			if let Err(e) = c.check() {
				if dbg {
					eprintln!("[modal-dbg] write_user_time({win}): X エラー: {e}");
				}
			} else if dbg {
				eprintln!("[modal-dbg] write_user_time({win}): time={time} 書き込み成功");
			}
		}
		Err(e) => {
			if dbg {
				eprintln!("[modal-dbg] write_user_time({win}): 要求失敗: {e}");
			}
		}
	}
	let _ = conn.flush();
}

/// 現在の X サーバー時刻を取得する (PropertyNotify トリック)。
///
/// 自分の窓でプロパティを変更し、発生した PropertyNotify イベントのタイム
/// スタンプを読む。X プロトコルには「現在時刻」を問い合わせる API がないた
/// め、この方法で取得する。約 200ms で諦める (失敗時は `None`)。
pub fn current_server_time(win: u32) -> Option<u32> {
	use x11rb::connection::Connection as _;
	use x11rb::protocol::xproto::ConnectionExt as _;
	use x11rb::protocol::xproto::EventMask;
	use x11rb::wrapper::ConnectionExt as _;
	let dbg = std::env::var("ERGON_MODAL_DEBUG").is_ok();
	let Ok((conn, _)) = x11rb::connect(None) else {
		if dbg {
			eprintln!("[modal-dbg] current_server_time({win}): 接続失敗");
		}
		return None;
	};
	// PropertyNotify を受信できるようにする。
	let Ok(attr) = conn.change_window_attributes(
		win,
		&x11rb::protocol::xproto::ChangeWindowAttributesAux::new()
			.event_mask(EventMask::PROPERTY_CHANGE),
	) else {
		if dbg {
			eprintln!("[modal-dbg] current_server_time({win}): 属性変更失敗");
		}
		return None;
	};
	if attr.check().is_err() {
		if dbg {
			eprintln!("[modal-dbg] current_server_time({win}): 属性変更が X エラー");
		}
		return None;
	}
	let _ = conn.flush();
	// プロパティを変更して PropertyNotify を発生させる。
	let Ok(name) = conn.intern_atom(false, b"WM_NAME") else { return None };
	let Ok(name) = name.reply() else { return None };
	let name = name.atom;
	let Ok(cp) = conn.change_property8(
		x11rb::protocol::xproto::PropMode::REPLACE,
		win,
		name,
		x11rb::protocol::xproto::AtomEnum::STRING,
		b"",
	) else { return None };
	let _ = cp.ignore_error();
	let _ = conn.flush();
	// イベントを待つ (タイムアウト付き)。
	let start = std::time::Instant::now();
	loop {
		if start.elapsed() > std::time::Duration::from_millis(200) {
			if dbg {
				eprintln!("[modal-dbg] current_server_time({win}): PropertyNotify タイムアウト");
			}
			return None;
		}
		match conn.poll_for_event() {
			Ok(Some(ev)) => {
				if let x11rb::protocol::Event::PropertyNotify(e) = ev {
					if e.window == win {
						return Some(e.time);
					}
				}
			}
			Ok(None) => std::thread::sleep(std::time::Duration::from_millis(2)),
			Err(_) => return None,
		}
	}
}

/// (診断) 窓のマップ状態と `_NET_WM_STATE` の遷移を 10ms 間隔で約 800ms 追
/// 踪し、変化したときだけ標準エラーへ出力する。`ERGON_MODAL_DEBUG=1` のとき
/// のみ動作する。マップ → withdraw → 再マップのような WM による二重管理
/// (ちらつきの原因) を切り分けるためのもの。
pub fn debug_trace_map_state(win: Option<u32>) {
	use x11rb::protocol::xproto::AtomEnum;
	use x11rb::protocol::xproto::ConnectionExt as _;
	use x11rb::protocol::xproto::MapState;
	let Some(win) = win else { return };
	if std::env::var("ERGON_MODAL_DEBUG").is_err() {
		return;
	}
	let Ok((conn, _screen)) = x11rb::connect(None) else { return };
	let Some(state_atom) = intern(&conn, b"_NET_WM_STATE") else { return };
	let snapshot = move |conn: &x11rb::rust_connection::RustConnection| -> Option<String> {
		let attrs = conn.get_window_attributes(win).ok()?.reply().ok()?;
		let map = match attrs.map_state {
			MapState::VIEWABLE => "Viewable",
			MapState::UNMAPPED => "Unmapped",
			_ => "Unviewable",
		};
		let mut states = String::new();
		if let Ok(r) = conn.get_property(false, win, state_atom, AtomEnum::ATOM, 0, 64) {
			if let Ok(r) = r.reply() {
				if let Some(vals) = r.value32() {
					for a in vals {
						if let Ok(c) = conn.get_atom_name(a) {
							if let Ok(n) = c.reply() {
								states.push_str(String::from_utf8_lossy(&n.name).trim_start_matches("_NET_WM_STATE_"));
								states.push(',');
							}
						}
					}
				}
			}
		}
		Some(format!("{map} [{states}]"))
	};
	let last = std::cell::RefCell::new(snapshot(&conn));
	let ticks = std::cell::Cell::new(0u32);
	let holder = std::rc::Rc::new(std::cell::RefCell::new(None::<slint::Timer>));
	let holder2 = holder.clone();
	let timer = slint::Timer::default();
	timer.start(
		slint::TimerMode::Repeated,
		std::time::Duration::from_millis(10),
		move || {
			let n = ticks.get() + 1;
			ticks.set(n);
			if n > 80 {
				if let Some(t) = holder2.borrow_mut().take() {
					t.stop();
				}
				return;
			}
			let cur = snapshot(&conn);
			if cur != *last.borrow() {
				eprintln!("[modal-dbg] trace +{}ms: {:?} -> {:?}", n * 10, last.borrow(), cur);
				*last.borrow_mut() = cur;
			}
		},
	);
	*holder.borrow_mut() = Some(timer);
}

/// (診断) 活性化要求の直前のフォーカス状況を出力する。Mutter は要求のタイ
/// ムスタンプがフォーカス窓の `_NET_WM_USER_TIME` より古いと活性化を拒否す
/// るため、拒否の切り分けに使う。`ERGON_MODAL_DEBUG=1` のときのみ出力する。
pub fn debug_dump_activation_context(child: u32, parent: Option<u32>, t: Option<u32>) {
	use x11rb::connection::Connection as _;
	use x11rb::protocol::xproto::AtomEnum;
	use x11rb::protocol::xproto::ConnectionExt as _;
	if std::env::var("ERGON_MODAL_DEBUG").is_err() {
		return;
	}
	let Ok((conn, _)) = x11rb::connect(None) else { return };
	let read_cardinal = |w: u32, name: &[u8]| -> Option<u32> {
		let Some(p) = intern(&conn, name) else { return None };
		let r = conn.get_property(false, w, p, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
		r.value32().and_then(|mut it| it.next())
	};
	let read_root_window_prop = |name: &[u8]| -> Option<u32> {
		let Some(p) = intern(&conn, name) else { return None };
		let root = conn.setup().roots[0].root;
		let r = conn.get_property(false, root, p, AtomEnum::WINDOW, 0, 8).ok()?.reply().ok()?;
		r.value32().and_then(|mut it| it.next())
	};
	let active = read_root_window_prop(b"_NET_ACTIVE_WINDOW");
	let parent_ut = parent.and_then(|p| read_cardinal(p, b"_NET_WM_USER_TIME"));
	let child_ut = read_cardinal(child, b"_NET_WM_USER_TIME");
	eprintln!(
		"[modal-dbg] 活性化要求: 子={child} t={:?} / _NET_ACTIVE_WINDOW={:?} / 親 user_time={:?} / 子 user_time={:?}",
		t, active, parent_ut, child_ut
	);
}

/// (診断) ダイアログ窓の WM プロパティとマップ状態を標準エラーへ出力する。
/// `ERGON_MODAL_DEBUG=1` のときのみ出力する。
pub fn debug_dump_dialog_props(win: u32) {
	use x11rb::connection::Connection as _;
	use x11rb::protocol::xproto::AtomEnum;
	use x11rb::protocol::xproto::ConnectionExt as _;
	use x11rb::protocol::xproto::MapState;
	if std::env::var("ERGON_MODAL_DEBUG").is_err() {
		return;
	}
	let Ok((conn, _screen)) = x11rb::connect(None) else { return };
	let read_atoms = |prop: u32| -> Vec<String> {
		let Ok(r) = conn.get_property(false, win, prop, AtomEnum::ATOM, 0, 1024)
			else { return vec![] };
		let Ok(r) = r.reply() else { return vec![] };
		r.value32()
			.map(|it| {
				it.map(|a| {
					conn.get_atom_name(a)
						.ok()
						.and_then(|c| c.reply().ok())
						.map(|n| String::from_utf8_lossy(&n.name).into_owned())
						.unwrap_or_else(|| format!("{a}"))
				})
				.collect()
			})
			.unwrap_or_default()
	};
	let dump = |name: &str, values: &[String]| {
		eprintln!("[modal-dbg] {name}: [{}]", values.join(", "));
	};
	let atom = |name: &[u8]| -> Option<u32> { intern(&conn, name) };
	let mut transient_for: Option<u32> = None;
	if let Some(p) = atom(b"WM_TRANSIENT_FOR") {
		let v: Vec<u32> = conn
			.get_property(false, win, p, AtomEnum::WINDOW, 0, 8)
			.ok()
			.and_then(|c| c.reply().ok())
			.and_then(|r| r.value32().map(|it| it.collect()))
			.unwrap_or_default();
		transient_for = v.first().copied();
		eprintln!("[modal-dbg] WM_TRANSIENT_FOR: {:?}", v);
	}
	// (診断) WM の管理窓一覧と transient 親の突き合わせ。Mutter は
	// WM_TRANSIENT_FOR の示す窓が管理表に無いと transient を無効化し、
	// 親基準の配置を行わない (screen.c の reload_transient_for 参照)。
	// この突き合わせで「親が管理表に無い」ことを即座に検出できる。
	if let Some(p) = atom(b"_NET_CLIENT_LIST") {
		let managed: Vec<u32> = conn
			.get_property(false, conn.setup().roots[0].root, p, AtomEnum::WINDOW, 0, 1024)
			.ok()
			.and_then(|c| c.reply().ok())
			.and_then(|r| r.value32().map(|it| it.collect()))
			.unwrap_or_default();
		if let Some(t) = transient_for {
			let in_list = managed.iter().any(|w| *w == t);
			// transient 親の状態 (管理表に無い場合はWMが扱えない)。
			let state = if in_list { "管理中" } else { "管理表に無い(!)" };
			eprintln!("[modal-dbg] transient 親 {t}: {state} (管理窓数={})", managed.len());
		}
		if let Some(ap) = atom(b"_NET_ACTIVE_WINDOW") {
			let active: Vec<u32> = conn
				.get_property(false, conn.setup().roots[0].root, ap, AtomEnum::WINDOW, 0, 8)
				.ok()
				.and_then(|c| c.reply().ok())
				.and_then(|r| r.value32().map(|it| it.collect()))
				.unwrap_or_default();
			eprintln!("[modal-dbg] _NET_ACTIVE_WINDOW: {:?} (管理窓: {:?})", active, managed);
		}
	}
	if let Some(p) = atom(b"_NET_WM_STATE") {
		dump("_NET_WM_STATE", &read_atoms(p));
	}
	if let Some(p) = atom(b"_NET_WM_WINDOW_TYPE") {
		dump("_NET_WM_WINDOW_TYPE", &read_atoms(p));
	}
	if let Some(p) = atom(b"_NET_WM_USER_TIME") {
		let v: Vec<u32> = conn
			.get_property(false, win, p, AtomEnum::CARDINAL, 0, 1)
			.ok()
			.and_then(|c| c.reply().ok())
			.and_then(|r| r.value32().map(|it| it.collect()))
			.unwrap_or_default();
		eprintln!("[modal-dbg] _NET_WM_USER_TIME: {:?}", v);
	}
	let attrs = conn.get_window_attributes(win);
	if let Ok(cookie) = attrs {
		if let Ok(rep) = cookie.reply() {
			eprintln!(
				"[modal-dbg] map_state={:?}",
				match rep.map_state {
					MapState::UNMAPPED => "Unmapped",
					MapState::UNVIEWABLE => "Unviewable",
					MapState::VIEWABLE => "Viewable",
					_ => "?",
				}
			);
		}
	}
	// 絶対位置 (ルート基準)。WM の配置結果の確認用。
	let root = conn.setup().roots[0].root;
	let geom = conn.get_geometry(win);
	let tr = conn.translate_coordinates(win, root, 0, 0);
	if let Ok(g) = geom {
		if let Ok(g) = g.reply() {
			if let Ok(t) = tr {
				if let Ok(t) = t.reply() {
					eprintln!(
						"[modal-dbg] geometry: {}x{} at root {},{}",
						g.width, g.height, t.dst_x, t.dst_y
					);
				}
			}
		}
	}
}

/// 親 (winit メイン窓) を選ぶ。GTK のリーダー窓は `_NET_WM_PID` を持つため
/// `linux_own_windows` に含まれてしまうが、未マップの InputOnly 窓なので、
/// 表示中 (Viewable) の窓を優先して選ぶ。見つからなければ先頭にフォールバック
/// する (最小化中など)。
fn pick_parent_window(conn: &x11rb::rust_connection::RustConnection) -> Option<u32> {
	use x11rb::protocol::xproto::{ConnectionExt as _, MapState};
	let mut first = None;
	for w in crate::platform::linux_own_windows(conn) {
		if first.is_none() {
			first = Some(w);
		}
		if let Some(rep) = conn.get_window_attributes(w).ok().and_then(|c| c.reply().ok()) {
			if rep.map_state == MapState::VIEWABLE {
				return Some(w);
			}
		}
	}
	first
}

/// ダイアログへ GDK のシートグラブ (キーボード + ポインタ) を試みる。ブロックは
/// 主に WM_TRANSIENT_FOR + `_NET_WM_STATE_MODAL` が担うので、これは best-effort
/// の保険。成功したら Seat を返し、Drop 側で ungrab する。
///
/// `owner_events = true` が必須。false にすると GDK は「同一アプリ内のイベント
/// もグラブ窓 (トップレベル) 基準でしか報告しない」ため、ダイアログ自身の OK 等
/// のボタンをクリックしても子ウィジェットに届かず `clicked` が出ない。すると
/// `response` が出ず `gtk_dialog_run()` が永久にブロックし、閉じないフリーズに
/// なる。true なら自分のアプリ内向けは通常どおり子へ届き (ボタンが反応し)、
/// 親 (別 X クライアント) への操作だけがグラブ窓へ回って塞がる。
fn grab_seat(dlg: &gtk::Dialog) -> Option<gtk::gdk::Seat> {
	let window = dlg.window()?;
	let seat = window.display().default_seat()?;
	let status = seat.grab(
		&window,
		gtk::gdk::SeatCapabilities::KEYBOARD | gtk::gdk::SeatCapabilities::POINTER,
		true,
		None,
		None,
		None,
	);
	(status == gtk::gdk::GrabStatus::Success).then_some(seat)
}

/// 表示中だけ親窓の `WM_PROTOCOLS` から `_NET_WM_PING` を外し、後で戻せるよう
/// 元の値と一緒に返す。
fn suppress_ping(
	conn: &x11rb::rust_connection::RustConnection,
	win: u32,
) -> Option<(u32, u32, Vec<u32>)> {
	use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode};
	use x11rb::wrapper::ConnectionExt as _;
	let wm_protocols = intern(conn, b"WM_PROTOCOLS")?;
	let net_wm_ping = intern(conn, b"_NET_WM_PING")?;
	let cur: Vec<u32> = conn
		.get_property(false, win, wm_protocols, AtomEnum::ATOM, 0, 1024)
		.ok()?
		.reply()
		.ok()?
		.value32()
		.map(|it| it.collect())
		.unwrap_or_default();
	if !cur.contains(&net_wm_ping) {
		return None;
	}
	let filtered: Vec<u32> = cur.iter().copied().filter(|a| *a != net_wm_ping).collect();
	let _ = conn
		.change_property32(PropMode::REPLACE, win, wm_protocols, AtomEnum::ATOM, &filtered)
		.map(|c| c.ignore_error());
	Some((win, wm_protocols, cur))
}

/// GTK ダイアログをモーダルにブロック実行して応答を返す。UI スレッドから呼ぶ
/// こと (GTK の再帰メインループがそのまま回る)。
pub fn run_dialog_blocking<D: IsA<gtk::Dialog>>(dlg: &D) -> gtk::ResponseType {
	use x11rb::connection::Connection as _;
	let dlg: &gtk::Dialog = dlg.upcast_ref();
	dlg.set_modal(true);

	// X11 が使えるとき: show (マップ) より前に親を transient にして、WM に
	// 親基準で配置させる。親窓 id は後の _NET_WM_PING 抑制にも使う。
	// (デバッグ用: NO_TRANSIENT=1 / NO_PING=1 / NO_GRAB=1 で各仕掛けを
	// 無効化して切り分けられる。通常はすべて有効。)
	let transient_off = std::env::var("NO_TRANSIENT").is_ok();
	let ping_off = std::env::var("NO_PING").is_ok();
	let grab_off = std::env::var("NO_GRAB").is_ok();
	let conn = x11rb::connect(None).ok().map(|(c, _)| c);
	let parent = conn.as_ref().and_then(pick_parent_window);
	if !transient_off {
		if let (Some(c), Some(p)) = (conn.as_ref(), parent) {
			// マップ前にダイアログのトップレベル X 窓へ transient を付ける
			// (WM が親基準で配置し、親より下に降ろせなくする)。
			for x in realize_dialog_xids(dlg, c) {
				set_transient_for(c, x, p);
			}
		}
	}
	let conn = conn.filter(|_| parent.is_some());

	dlg.show();
	wait_viewable(dlg.upcast_ref(), 1500);
	let ping_restore = if ping_off {
		None
	} else {
		match (conn.as_ref(), parent) {
			(Some(c), Some(p)) => {
				let r = suppress_ping(c, p);
				// x11rb は変更をクライアント側にバッファするため、ここで必ず
				// サーバーへ送っておかないと WM が _NET_WM_PING を送り続けて
				// 親が「応答なし」扱いになる。
				let _ = c.flush();
				r
			}
			_ => None,
		}
	};

	let seat = if grab_off { None } else { grab_seat(dlg) };

	// 再帰メインループを実行して応答を待つ。
	let ret = dlg.run();

	// 後始末 (順序重要): グラブ解除 → PING 復帰 (flush)。
	if let Some(seat) = seat {
		seat.ungrab();
	}
	if let Some(c) = conn.as_ref() {
		if let Some((win, prop, orig)) = ping_restore {
			use x11rb::protocol::xproto::{AtomEnum, PropMode};
			use x11rb::wrapper::ConnectionExt as _;
			let _ = c
				.change_property32(PropMode::REPLACE, win, prop, AtomEnum::ATOM, &orig)
				.map(|c| c.ignore_error());
		}
		let _ = c.flush();
	}

	// close() は隠すだけで GtkWindow/X ウィンドウが生き残り、X の入力
	// フォーカスが隠れた窓に残ってキーボード入力が死ぬため、必ず破棄する。
	unsafe { dlg.destroy() };

	// ここから先はメインループが GDK を回さない。モーダルグラブの解除や
	// ウィンドウ破棄の要求が GDK 側の X 出力バッファに残ったままだと X サー
	// バーに届かず、グラブが解放されないままキーボード入力が死ぬため、明示
	// 的にフラッシュする。
	if let Some(disp) = gtk::gdk::Display::default() {
		disp.flush();
	}

	// 破棄後、X の入力フォーカスを親窓へ戻す。WM がフォーカスを復帰させな
	// い環境では、キーボード入力がアプリへ届かなくなり、閉じた直後からフリー
	// ズしたように見える。
	if let (Some(c), Some(p)) = (conn.as_ref(), parent) {
		use x11rb::protocol::xproto::{ConnectionExt as _, InputFocus};
		let _ = c
			.set_input_focus(InputFocus::PARENT, p, 0u32)
			.map(|c| c.ignore_error());
		let _ = c.flush();
	}
	ret
}

/// ダイアログにボタンを追加する。`key` は Alt+キーのアクセラレータ (単一
/// 文字)。ラベル中の "(B)" 等は常に見えるアクセスキー表示で、Alt+<key> が
/// 実際に効く (ニーモニックの下線はテーマ/設定によって隠れるため、表示は
/// リテラルにしてある)。
pub fn add_button<D: IsA<gtk::Dialog>>(
	dlg: &D,
	group: &gtk::AccelGroup,
	label: &str,
	key: &str,
	id: gtk::ResponseType,
) -> gtk::Widget {
	let btn = dlg.add_button(label, id);
	let keyval = gtk::gdk::keys::Key::from_name(key);
	if *keyval != 0 {
		btn.add_accelerator(
			"clicked",
			group,
			*keyval,
			gtk::gdk::ModifierType::MOD1_MASK,
			gtk::AccelFlags::VISIBLE,
		);
	}
	btn
}
