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
//! ため、マップ前にダイアログの X 窓へ `WM_TRANSIENT_FOR` を直接設定する
//! (`_NET_WM_PID` が付いた自分のトップレベル窓から親を選ぶが、GTK は realize
//! 時に補助窓も作るので新しく生えた窓すべてに付ける)。これで WM が親の中央に
//! 置き、`_NET_WM_STATE_MODAL` (set_modal 由来) と合わせて親をフォーカス/操作
//! できないモーダルなペアとして扱う。
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

/// root の直接の子ウィンドウ id を列挙する。realize 直後 (未マップ) の
/// ダイアログ X 窓は WM に reparent される前なので、差分で一意に拾える。
fn root_children(conn: &x11rb::rust_connection::RustConnection) -> Vec<u32> {
	use x11rb::connection::Connection;
	use x11rb::protocol::xproto::ConnectionExt;
	let root = conn.setup().roots[0].root;
	conn.query_tree(root)
		.ok()
		.and_then(|c| c.reply().ok())
		.map(|r| r.children)
		.unwrap_or_default()
}

/// ダイアログを realize (X 窓生成、まだマップしない) し、新しく生えた X 窓 id を
/// 返す。WM が親基準で配置できるよう、show (マップ) より前に transient を
/// 付けたいので、ここで先に X 窓を得る。GTK は realize 時にトップレベルとは別に
/// 補助の X 窓も作る (どれが WM に管理されるか事前には分からない) ため、新しく
/// 生えた窓すべてに transient を付ける。
fn realize_dialog_xids(
	dlg: &gtk::Dialog,
	conn: &x11rb::rust_connection::RustConnection,
) -> Vec<u32> {
	let before = root_children(conn);
	dlg.realize();
	root_children(conn).into_iter().filter(|w| !before.contains(w)).collect()
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
			// GTK は realize 時にトップレベルの他に補助窓も作るため、新しく生えた
			// X 窓すべてに transient を付ける (WM が管理するのはその中の 1 つ)。
			for x in realize_dialog_xids(dlg, c) {
				set_transient_for(c, x, p);
			}
		}
	}
	let conn = conn.filter(|_| parent.is_some());

	dlg.show();
	wait_viewable(dlg.upcast_ref(), 1500);
	// TEMP DEBUG: OK ボタンの絶対座標と response の発火を確認する (フリーズ調査用)。
	{
		use gtk::prelude::*;
		fn find_button(w: &gtk::Widget) -> Option<gtk::Button> {
			if let Some(b) = w.downcast_ref::<gtk::Button>() {
				return Some(b.clone());
			}
			if let Some(c) = w.dynamic_cast_ref::<gtk::Container>() {
				for ch in c.children() {
					if let Some(b) = find_button(&ch) {
						return Some(b);
					}
				}
			}
			None
		}
		let btn = find_button(dlg.upcast_ref());
		if let Some(btn) = btn {
			gtk::glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
				let alloc = btn.allocation();
				if let Some(win) = btn.window() {
					let (wx, wy) = win.position();
					eprintln!(
						"[gtk-dbg] OK-POS x={} y={} (w={} h={})",
						wx + alloc.x(),
						wy + alloc.y(),
						alloc.width(),
						alloc.height()
					);
				}
				true.into()
			});
		}
		dlg.connect_response(move |_, r| {
			eprintln!("[gtk-dbg] response fired: {r:?}");
		});
	}
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

	dlg.close();

	// ダイアログを閉じた後、X の入力フォーカスを親窓へ戻す。
	// WM がフォーカスを復帰させない環境では、キーボード入力がアプリへ届か
	// なくなり、閉じた直後からフリーズしたように見える。
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
