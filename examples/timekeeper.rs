// じかん君 — ergon-slint-controls の全機能を使うサンプル アプリ。
//
// ビルド: cargo run --example timekeeper
// (Linux では libgtk-3-dev + pkg-config が必要)
//
// 使っているライブラリの機能:
// - Slint 部品: メニューバー / ドロップダウン / SmallButton / ModeToggleButton /
//   CosmicCheckBox / SpinBox / Label (& アクセスキー) / AppTooltip —
//   ui/timekeeper.slint / ui/timekeeper-export.slint を参照
// - menu: & 記法のパース (parse_access_key / access_key_char) と
//   MenuAccessKey.parse への差し込み
// - AccessKeyHub: ラベルのアクセスキー → access-key-pressed → スピンボックスへ
//   フォーカス
// - dialogs: フォルダ選択 / Yes-No 確認 / カスタム ボタン / 情報表示 /
//   表示中の親入力遮断 / 自前の遮断区間 (dialog_push / dialog_pop)
// - modal_window: 「CSV を書き出す」ダイアログの子ウィンドウ方式 + ウィンドウ内
//   オーバーレイ方式 (ModalSlot / ModalChild / OverlayHost / overlay_key_filter)
// - platform: タイトルバー テーマ / 修飾キーの直接読み / (Windows) ウィンドウ
//   アイコン / owner HWND
// - cursor: ドラッグ中の自前カーソル / 十字カーソル / (Windows) WM_SETCURSOR の
//   維持と非クライアント クリック
//
// 外部アプリでは .slint の部品を
// `import { ... } from "@ergon_slint_controls"` で参照する。このサンプルは
// クレート内にあるため同じコンパイル単位で直接 import しているが、API の形は
// 同じ (README の「参照方法の要点」を参照)。

use ergon_slint_controls::menu::{access_key_char, parse_access_key, pressed_char};
use ergon_slint_controls::{AccessKeyHub, MenuAccessKey, MenuOpenHub, SessionEntry, TimeKeeperExport, TimeKeeperWindow};
use modal_window::OverlayHost as _;
use slint::{winit_030::{winit, EventResult, WinitWindowAccessor}, ComponentHandle as _, Model as _};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

/// (メニュー index, 行 index) → コマンド。区切り行は None。
/// .slint 側の file-actions / task-actions / help-actions と同じ表。
fn menu_action(menu: i32, row: usize) -> Option<String> {
	const FILE: [&str; 4] = ["--folder", "--csv", "", "--exit"];
	const TASK: [&str; 4] = ["--start", "--stop", "", "--clear"];
	const HELP: [&str; 2] = ["--help", "--about"];
	match menu {
		0 => FILE.get(row).filter(|s| !s.is_empty()).map(|s| s.to_string()),
		1 => TASK.get(row).filter(|s| !s.is_empty()).map(|s| s.to_string()),
		2 => HELP.get(row).filter(|s| !s.is_empty()).map(|s| s.to_string()),
		_ => None,
	}
}

/// メニューコマンドの実行 (ポップアップを閉じてから処理する)。
fn apply_menu_action(ui: &TimeKeeperWindow, action: &str) {
	ui.global::<MenuOpenHub>().invoke_close();
	ui.invoke_menu_item_clicked(action.into());
}

/// `from` の隣 (step = +1 で下 / -1 で上) から探索を始め、次の有効行
/// (区切り / 無効行を読み飛ばす) を返す。端は折り返す。`from < 0` なら
/// step > 0 なら先頭から、step < 0 なら末尾から探す。
fn next_enabled_row(
	items: &slint::ModelRc<ergon_slint_controls::MenuItemView>,
	from: i32,
	step: i32,
) -> i32 {
	let n = items.row_count() as i32;
	if n == 0 {
		return -1;
	}
	let mut i = if from < 0 {
		if step > 0 { -1 } else { n }
	} else {
		from
	};
	for _ in 0..n {
		i += step;
		if i < 0 {
			i = n - 1;   // 上端で折り返す
		} else if i >= n {
			i = 0;       // 下端で折り返す
		}
		if items.row_data(i as usize).map_or(false, |v| v.entry.enabled && !v.entry.is_separator) {
			return i;
		}
	}
	-1
}

/// 「書き出す」ダイアログのラッパ。modal_window の ModalChild トレイトを実装
/// する (window / show / hide / accept / cancel の 5 メソッドのみ)。
struct ExportDialog(TimeKeeperExport);

impl modal_window::ModalChild for ExportDialog {
	fn window(&self) -> &slint::Window {
		self.0.window()
	}
	fn show(&self) -> Result<(), slint::PlatformError> {
		self.0.show()
	}
	fn hide(&self) -> Result<(), slint::PlatformError> {
		self.0.hide()
	}
	fn accept(&self) {
		self.0.invoke_ok();   // Enter を「書き出す」へ
	}
	fn cancel(&self) {
		self.0.invoke_cancel();   // Esc / 閉じるボタン / CloseRequested をキャンセルへ
	}
}

/// ウィンドウ内オーバーレイ方式の「書き出す」パネルのホスト。
/// 表示の切り替えと確定 / キャンセルの転送だけを書き、二重オープン ガードや
/// Esc/Enter の読み替えは modal_window が行う。
struct ExportOverlayHost {
	weak: slint::Weak<TimeKeeperWindow>,
	do_export: Rc<dyn Fn()>,
}

impl modal_window::OverlayHost for ExportOverlayHost {
	fn is_open(&self) -> bool {
		self.weak.upgrade().map_or(false, |ui| ui.get_export_overlay_visible())
	}
	fn open(&self) {
		if let Some(ui) = self.weak.upgrade() {
			ui.set_export_overlay_visible(true);
		}
	}
	fn close(&self) {
		if let Some(ui) = self.weak.upgrade() {
			ui.set_export_overlay_visible(false);
		}
	}
	fn accept(&self) {
		(self.do_export)();
		self.close();
	}
	fn cancel(&self) {
		self.close();
	}
}

/// ドラッグ中の自前カーソル (16x16 のつかみどり)。
fn grab_cursor() -> ergon_slint_controls::cursor::CursorBitmap {
	const W: u32 = 16;
	const H: u32 = 16;
	let mut pixels = vec![0u32; (W * H) as usize];
	for y in 0..H {
		for x in 0..W {
			let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
			if d <= 6.0 {
				pixels[(y * W + x) as usize] = 0xE8E07020;   // AARRGGBB (オレンジの丸)
			}
			if d <= 2.5 {
				pixels[(y * W + x) as usize] = 0xF8FFFFFF;   // 中心は白
			}
		}
	}
	ergon_slint_controls::cursor::CursorBitmap {
		width: W,
		height: H,
		hotspot_x: 7,
		hotspot_y: 7,
		pixels,
	}
}

/// 秒数を "25:05" 形式に整える。
fn fmt_mmss(secs: i64) -> String {
	format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn main() -> Result<(), slint::PlatformError> {
	// winit バックエンドの準備は modal_window へ委譲する。子ウィンドウ モード
	// では、ここでモーダル子ダイアログの生成属性フック (Windows: ダイアログ枠 +
	// 所有ウィンドウ / X11: _NET_WM_WINDOW_TYPE_DIALOG) が仕込まれる。
	modal_window::prepare_backend()?;
	let ui = TimeKeeperWindow::new()?;

	// `&` 記法のパースをライブラリ実装へ差し込む (必須)。記法の解釈 (Win32
	// 標準: &X = アクセスキー, && = リテラルの &) は ergon_slint_controls::menu
	// が担う。描画 (下線付きラベル) と Rust 側のキー発火が同じ関数を読むため
	// 表示と動作が一致する。
	ui.global::<MenuAccessKey>().on_parse(|title| {
		let p = parse_access_key(&title);
		ergon_slint_controls::AccessKeyParts {
			before: p.before.into(),
			key: p.key.into(),
			after: p.after.into(),
		}
	});

	// ---------- アプリの状態 ----------
	// ログの保存先 (「ログの保存先フォルダ...」で設定)。
	let save_dir: Rc<std::cell::RefCell<Option<PathBuf>>> = Rc::new(std::cell::RefCell::new(None));
	// 今日のセッション一覧 (UI の sessions モデル)。
	let sessions = Rc::new(slint::VecModel::<SessionEntry>::default());
	ui.set_sessions(sessions.clone().into());

	// 起動時の修飾キーを OS から直接読む。Shift 起動なら作業名を前回値の
	// ままにせず空欄から始める (デモ)。
	let (shift_os, _ctrl_os) = ergon_slint_controls::platform::os_modifier_keys();
	if shift_os {
		ui.set_task_name("".into());
	}

	// ---------- 計測タイマ (1 秒ごと) ----------
	let timer = slint::Timer::default();
	{
		let weak = ui.as_weak();
		timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
			let Some(ui) = weak.upgrade() else { return };
			if !ui.get_running() {
				return;
			}
			let e = ui.get_elapsed_secs() + 1;
			ui.set_elapsed_secs(e);
			// 表示 (経過 / 残り)
			let t = if ui.get_show_remaining() {
				(ui.get_goal_minutes() as i64) * 60 - e as i64
			} else {
				e as i64
			};
			ui.set_clock_text(fmt_mmss(t.max(0)).into());
			// 休憩の合図
			if ui.get_break_reminder() {
				let every = ui.get_break_minutes().max(1) as i64 * 60;
				if e as i64 % every == 0 {
					ui.set_hint_text("休憩の時間です。少し立ち上がりましょう。".into());
				}
			}
		});
	}

	// ---------- 開始 / 停止 ----------
	let start_stop: Rc<dyn Fn()> = {
		let weak = ui.as_weak();
		let sessions = sessions.clone();
		Rc::new(move || {
			let Some(ui) = weak.upgrade() else { return };
			if ui.get_running() {
				// 停止: 一覧に加える
				let e = ui.get_elapsed_secs();
				sessions.push(SessionEntry {
					start: clock_hhmm().into(),
					task: ui.get_task_name(),
					secs: e,
					label: fmt_mmss(e as i64).into(),
				});
				ui.set_running(false);
				ui.set_elapsed_secs(0);
				ui.set_clock_text("00:00:00".into());
				ui.set_hint_text("停止しました。".into());
			} else {
				ui.set_elapsed_secs(0);
				ui.set_running(true);
				ui.set_hint_text("計測を開始しました。".into());
			}
		})
	};

	// ---------- CSV の書き出し ----------
	// オーバーレイ パネルの確定と子ウィンドウの OK の両方から呼ばれる。
	let do_export: Rc<dyn Fn()> = {
		let weak = ui.as_weak();
		let sessions = sessions.clone();
		let save_dir = save_dir.clone();
		Rc::new(move || {
			let Some(ui) = weak.upgrade() else { return };
			// 保存先が未設定なら選んでもらう。
			let dir = save_dir.borrow().clone().or_else(|| {
				ergon_slint_controls::dialogs::prompt_folder("ログの保存先フォルダ", None)
			});
			let Some(dir) = dir else { return };
			*save_dir.borrow_mut() = Some(dir.clone());
			let path = dir.join("sessions.csv");

			// 上書き確認 (Yes/No)。
			if path.exists() {
				let go = matches!(
					ergon_slint_controls::dialogs::message_box(
						"上書き確認",
						"同名のファイルがあります。上書きしますか？",
						ergon_slint_controls::dialogs::MessageButtons::YesNo,
					),
					Some(ergon_slint_controls::dialogs::MessageResponse::Yes)
				);
				if !go {
					return;
				}
			}

			// CSV を組み立てる (区切り / ヘッダー / BOM は UI のオプション)。
			let comma = ui.get_export_comma();
			let sep = if comma { "," } else { "\t" };
			let mut csv = String::new();
			if ui.get_export_bom() {
				csv.push('\u{FEFF}');
			}
			if ui.get_export_header() {
				csv.push_str(&format!("開始{sep}タスク{sep}秒\n"));
			}
			let n = sessions.row_count();
			for i in 0..n {
				let Some(e) = sessions.row_data(i) else { continue };
				csv.push_str(&format!("{}{sep}\"{}\"{sep}{}\n", e.start, e.task.replace('"', "\"\""), e.secs));
			}

			// 書き出し中は自前の入力遮断区間を作る (dialog_push / dialog_pop)。
			// この区間は dialog_active() を true にし、stale_input_pending() が
			// 閉じ直後の入力を捨てる。
			ergon_slint_controls::dialogs::dialog_push();
			let result = std::fs::write(&path, csv);
			ergon_slint_controls::dialogs::dialog_pop();

			match result {
				Ok(()) => ergon_slint_controls::dialogs::show_message_box(
					"書き出し完了",
					&format!("{} 件を\n{} に書き出しました。", n, path.display()),
					ergon_slint_controls::dialogs::MessageIcon::Info,
				),
				Err(err) => ergon_slint_controls::dialogs::show_message_box(
					"書き出しエラー",
					&format!("書き出せませんでした。\n\n{err}"),
					ergon_slint_controls::dialogs::MessageIcon::Error,
				),
			}
		})
	};

	// ---------- 「書き出す」ダイアログ ----------
	let export_slot: modal_window::ModalSlot = Default::default();
	let export_overlay_host = Rc::new(ExportOverlayHost {
		weak: ui.as_weak(),
		do_export: do_export.clone(),
	});
	{
		let host = export_overlay_host.clone();
		ui.on_export_ok(move || host.accept());
	}
	{
		let host = export_overlay_host.clone();
		ui.on_export_cancel(move || host.cancel());
	}

	/// 子ウィンドウ方式で「書き出す」ダイアログを開く。
	fn open_export(
		main: &TimeKeeperWindow,
		slot: &modal_window::ModalSlot,
		do_export: Rc<dyn Fn()>,
	) {
		if modal_window::MODAL_MODE.is_in_window_overlay() {
			// メインウィンドウ内のオーバーレイで開く (二重オープン ガード付き)。
			let host = ExportOverlayHost { weak: main.as_weak(), do_export };
			modal_window::open_overlay(&host);
			return;
		}
		// 子ウィンドウ方式: 生成と値の同期はこちら、表示と制御はライブラリ。
		if slot.borrow().is_some() {
			return;   // 二重オープン ガード (ライブラリ側にも同じガードがある)
		}
		// 次のウィンドウ生成がモーダル子ダイアログだとライブラリへ伝える。
		#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
		modal_window::mark_next_window_as_child_dialog();

		let Ok(dlg) = TimeKeeperExport::new() else { return };
		// 表示直前に現在値を同期する。
		dlg.set_comma(main.get_export_comma());
		dlg.set_header(main.get_export_header());
		dlg.set_bom(main.get_export_bom());

		// 確定: 値を書き戻してから書き出す (破棄はライブラリ)。
		{
			let slot = slot.clone();
			let main_weak = main.as_weak();
			let weak = dlg.as_weak();
			let do_export = do_export.clone();
			dlg.on_ok(move || {
				if let (Some(d), Some(m)) = (weak.upgrade(), main_weak.upgrade()) {
					m.set_export_comma(d.get_comma());
					m.set_export_header(d.get_header());
					m.set_export_bom(d.get_bom());
					modal_window::close_child_modal(&slot, m.window());
					do_export();
				}
			});
		}
		// キャンセル: 値を捨てて閉じる。
		{
			let slot = slot.clone();
			let main_weak = main.as_weak();
			dlg.on_cancel(move || {
				if let Some(m) = main_weak.upgrade() {
					modal_window::close_child_modal(&slot, m.window());
				}
			});
		}
		// 表示とモーダル化 (Esc/Enter・中央配置・破棄) はライブラリへ任せる。
		modal_window::open_child_modal(slot, main, Rc::new(ExportDialog(dlg)));
	}

	// ---------- メニューのコールバック ----------
	{
		let weak = ui.as_weak();
		let slot = export_slot.clone();
		let save_dir = save_dir.clone();
		let sessions = sessions.clone();
		let start_stop = start_stop.clone();
		ui.on_menu_item_clicked(move |id| {
			let Some(ui) = weak.upgrade() else { return };
			// ブロックしない即時処理: メニューを閉じた更新と同じフレームに
			// まとまる (表示更新は 1 回だけ)。
			match id.as_str() {
				"--start" | "--stop" => start_stop(),
				"--clear" => {
					while sessions.row_count() > 0 {
						sessions.remove(0);
					}
					ui.set_hint_text("一覧を空にしました。".into());
				}
				// ネイティブ ダイアログを出す処理: メニューを閉じた描画を
				// 先に済ませてから実行する。
				_ => {
					// 呼び出しのたびに必要な状態を clone して遅延クロージャへ渡す。
					let weak2 = weak.clone();
					let save_dir = save_dir.clone();
					let slot = slot.clone();
					let do_export = do_export.clone();
					let start_stop = start_stop.clone();
					ergon_slint_controls::menu::defer_action(&ui, move || {
						let Some(ui) = weak2.upgrade() else { return };
						match id.as_str() {
						"--folder" => {
							if let Some(dir) = ergon_slint_controls::dialogs::prompt_folder("ログの保存先フォルダ", None) {
								*save_dir.borrow_mut() = Some(dir.clone());
								ui.set_hint_text(slint::format!("保存先: {}", dir.display()).into());
							}
						}
						"--csv" => open_export(&ui, &slot, do_export.clone()),
						"--exit" => {
							// 実行中ならカスタム ボタンで尋ねる (選択 index を返す)。
							if ui.get_running() {
								match ergon_slint_controls::dialogs::message_box_custom(
									"終了の確認",
									"計測が実行中です。どうしますか？",
									&["停止して終了 (&Q)", "破棄して終了 (&D)", "キャンセル"],
									Some(2),   // 既定 (Enter) はキャンセル
								) {
									Some(0) => {
										start_stop();
										slint::quit_event_loop().ok();
									}
									Some(1) => {
										ui.set_running(false);
										slint::quit_event_loop().ok();
									}
									_ => {}
								}
							} else {
								slint::quit_event_loop().ok();
							}
						}
						"--help" => ergon_slint_controls::dialogs::show_message_box(
							"操作方法",
							"Alt+F/T/H でメニュー、矢印キーで移動、Enter で決定。\n\
							 メニューを開いている間は各項目の (&X) の文字でも発火します。\n\
							 R で休憩の合図の分を入力欄へ、B/P で開始 / 停止。\n\
							 時計カードはドラッグで移動できます。",
							ergon_slint_controls::dialogs::MessageIcon::Info,
						),
						"--about" => ergon_slint_controls::dialogs::show_message_box(
							"バージョン情報",
							"じかん君 サンプル 1.0\nergon-slint-controls のデモです。",
							ergon_slint_controls::dialogs::MessageIcon::Info,
						),
						_ => {}
						}
					});
				}
			}
		});
	}

	{
		let weak = ui.as_weak();
		ui.on_break_changed(move |v| {
			if let Some(ui) = weak.upgrade() {
				ui.set_hint_text(slint::format!("休憩の合図を {} 分にしました。", v).into());
			}
		});
	}
	{
		// ドラッグ中は自前カーソル、離したら標準へ戻す。
		ui.on_drag_changed(move |dragging| {
			if dragging {
				ergon_slint_controls::cursor::set_custom_cursor(1, grab_cursor);
			} else {
				ergon_slint_controls::cursor::set_system_cursor(
					ergon_slint_controls::cursor::SystemCursor::Default,
				);
			}
		});
	}
	{
		// 進捗バーの hover は十字カーソル (調整の暗示)。
		ui.on_bar_hover(move |hovering| {
			ergon_slint_controls::cursor::set_system_cursor(if hovering {
				ergon_slint_controls::cursor::SystemCursor::Crosshair
			} else {
				ergon_slint_controls::cursor::SystemCursor::Default
			});
		});
	}

	// ---------- キーボード処理 (winit イベント フック) ----------
	let weak = ui.as_weak();
	let alt_state = AtomicBool::new(false);
	ui.window().on_winit_window_event(move |_win, event| -> EventResult {
		if let winit::event::WindowEvent::ModifiersChanged(mods) = event {
			let mods = mods.state();
			alt_state.store(mods.alt_key(), Relaxed);
			// Alt 押下中はメニューのアクセスキーに下線を表示する。
			if let Some(ui) = weak.upgrade() {
				ui.set_menu_alt_held(mods.alt_key());
			}
		}

		// ネイティブ ダイアログが表示中 (または直後に閉じた) ときは、
		// 親ウィンドウへ届いた入力をすべて握り潰す。
		#[cfg(not(windows))]
		if ergon_slint_controls::dialogs::dialog_active()
			|| ergon_slint_controls::dialogs::stale_input_pending()
		{
			use winit::event::WindowEvent::*;
			if matches!(
				event,
				KeyboardInput { .. }
					| ModifiersChanged(_)
					| Ime(_)
					| Focused(_)
					| CursorMoved { .. }
					| CursorEntered { .. }
					| CursorLeft { .. }
					| MouseWheel { .. }
					| MouseInput { .. }
					| Touch(_)
					| DroppedFile(_)
			) {
				return EventResult::PreventDefault;
			}
		}

		// Windows: owner HWND の確保とカーソル維持 (生成後 1 回実効があればよい)。
		#[cfg(windows)]
		{
			use ergon_slint_controls::cursor::install_override;
			use ergon_slint_controls::dialogs::capture_dialog_owner;
			use ergon_slint_controls::platform::{install_window_icon, main_window_hwnd};
			capture_dialog_owner(_win);
			let hwnd = main_window_hwnd(_win);
			if !hwnd.is_null() {
				install_override(_win);
				install_window_icon(hwnd, 1);   // exe 埋め込みリソースの .ico
			}
		}

		// Windows: 非クライアント領域のクリックを拾ったら握り潰す。
		#[cfg(windows)]
		if let winit::event::WindowEvent::MouseInput { .. } = event {
			if ergon_slint_controls::cursor::take_non_client_click() {
				return EventResult::PreventDefault;
			}
		}

		// ウィンドウの外がクリックされた (非活性になった) ときもメニューを閉じる。
		if let winit::event::WindowEvent::Focused(false) = event {
			if let Some(ui) = weak.upgrade() {
				ui.global::<MenuOpenHub>().invoke_close();
			}
		}

		let winit::event::WindowEvent::KeyboardInput { event: key_event, .. } = event else {
			return EventResult::Propagate;
		};
		// 離し (Released) は処理しない (押下だけで判定する)。
		if key_event.state != winit::event::ElementState::Pressed {
			return EventResult::Propagate;
		}
		let code = match key_event.physical_key {
			winit::keyboard::PhysicalKey::Code(c) => c,
			_ => return EventResult::Propagate,
		};
		use winit::keyboard::KeyCode;

		// オーバーレイ式モーダル表示中のキー (Esc=キャンセル / Enter=確定、
		// 他は握り潰し) は modal_window へ委譲する。
		if modal_window::overlay_key_filter(&*export_overlay_host, code, key_event.repeat) {
			return EventResult::PreventDefault;
		}

		let open = weak.upgrade().map(|ui| ui.global::<MenuOpenHub>().get_open()).unwrap_or(-1);

		// --- Alt+文字: バー項目のアクセスキーでメニューを開閉 ---
		if !key_event.repeat && alt_state.load(Relaxed) {
			if let Some(ui) = weak.upgrade() {
				let want = pressed_char(key_event).and_then(|p| {
					[
						(0usize, ui.get_file_menu_title()),
						(1, ui.get_task_menu_title()),
						(2, ui.get_help_menu_title()),
					]
					.into_iter()
					.find(|(_, t)| access_key_char(t) == Some(p.to_ascii_uppercase()))
					.map(|(id, _)| id)
				});
				if let Some(want) = want {
					let hub = ui.global::<MenuOpenHub>();
					if hub.get_open() == want as i32 {
						hub.invoke_close();
					} else {
						hub.set_open(want as i32);
						// キーボードで開いたときは最初の有効行を初期選択にする
						// (区切り / 無効行は読み飛ばす。モデルが唯一の情報源)。
						let items = match want {
							1 => ui.get_task_items(),
							2 => ui.get_help_items(),
							_ => ui.get_file_items(),
						};
						ui.set_menu_index(next_enabled_row(&items, -1, 1));
					}
					return EventResult::PreventDefault;
				}
			}
		}

		// --- メニュー開閉中: 項目のアクセスキー発火 ---
		if !key_event.repeat && open >= 0 {
			if let Some(ui) = weak.upgrade() {
				let items = match open {
					0 => ui.get_file_items(),
					1 => ui.get_task_items(),
					2 => ui.get_help_items(),
					_ => return EventResult::Propagate,
				};
				if let Some(pressed) = pressed_char(key_event) {
					let row = (0..items.row_count()).find(|&i| {
						items.row_data(i).map_or(false, |v| {
							v.entry.enabled
								&& !v.entry.is_separator
								&& access_key_char(&v.entry.title) == Some(pressed.to_ascii_uppercase())
						})
					});
					if let Some(row) = row {
						// 行 → コマンドは Enter と同じ表を使う (menu_action)。
						if let Some(action) = menu_action(open, row) {
							let weak = weak.clone();
							let _ = slint::invoke_from_event_loop(move || {
								if let Some(ui) = weak.upgrade() {
									apply_menu_action(&ui, &action);
								}
							});
							return EventResult::PreventDefault;
						}
					}
				}
			}
		}

		// --- メニュー開閉中: 上下移動 (有効行のみ、端は折り返す) ---
		if open >= 0 && matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown) {
			let Some(ui) = weak.upgrade() else { return EventResult::PreventDefault };
			let items = match open {
				0 => ui.get_file_items(),
				1 => ui.get_task_items(),
				2 => ui.get_help_items(),
				_ => return EventResult::PreventDefault,
			};
			let step = if matches!(code, KeyCode::ArrowDown) { 1 } else { -1 };
			ui.set_menu_index(next_enabled_row(&items, ui.get_menu_index(), step));
			return EventResult::PreventDefault;
		}

		// --- メニュー開閉中: 左右でメニュー切替 / Enter で決定 / Esc で閉じる ---
		if open >= 0 && matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
			if let Some(ui) = weak.upgrade() {
				let next = if matches!(code, KeyCode::ArrowRight) { (open + 1) % 3 } else { (open + 2) % 3 };
				ui.global::<MenuOpenHub>().set_open(next);
				let items = match next {
					1 => ui.get_task_items(),
					2 => ui.get_help_items(),
					_ => ui.get_file_items(),
				};
				ui.set_menu_index(next_enabled_row(&items, -1, 1));
			}
			return EventResult::PreventDefault;
		}
		if open >= 0 && matches!(code, KeyCode::Enter | KeyCode::Space) {
			if let Some(ui) = weak.upgrade() {
				let idx = ui.get_menu_index();
				if idx >= 0 {
					if let Some(action) = menu_action(open, idx as usize) {
						apply_menu_action(&ui, &action);
					}
				}
				return EventResult::PreventDefault;
			}
		}
		if open >= 0 && matches!(code, KeyCode::Escape) {
			if let Some(ui) = weak.upgrade() {
				ui.global::<MenuOpenHub>().invoke_close();
				ui.set_menu_index(-1);
			}
			return EventResult::PreventDefault;
		}

		// --- 開始 / 停止ボタンのアクセスキー (Alt 不要) ---
		// Slint にボタン用のアクセスキー機構は無いため、タイトルの &X を
		// 自分で読んで発火させる (単一ソース。ボタンのタイトルは実行中で
		// 入れ替わるため、プロパティ経由で同じ文字列を読む)。
		if !key_event.repeat && open < 0 {
			if let Some(pressed) = pressed_char(key_event).map(|c| c.to_ascii_uppercase()) {
				if let Some(ui) = weak.upgrade() {
					if access_key_char(&ui.get_start_title()) == Some(pressed) {
						start_stop();
						return EventResult::PreventDefault;
					}
				}
			}
		}

		// --- ラベルのアクセスキー (休憩の合図 &R) → AccessKeyHub へ通知 ---
		// 一致する Label が access-key-pressed を発火し、ハンドラで
		// スピンボックスへフォーカスする。
		if !key_event.repeat && open < 0 {
			if let Some(ui) = weak.upgrade() {
				if let Some(pressed) = pressed_char(key_event) {
					if access_key_char(&ui.get_break_title()) == Some(pressed.to_ascii_uppercase()) {
						ui.global::<AccessKeyHub>()
							.invoke_press(pressed.to_ascii_uppercase().to_string().into());
						return EventResult::PreventDefault;
					}
				}
			}
		}

		EventResult::Propagate
	});

	// ---------- カーソルの下準備 (どの OS も同じ API) ----------
	// winit ウィンドウ (Arc) をライブラリへ渡しておくとカーソル制御に使われる。
	// set_file_cursor は .cur ファイルから取り付ける (Windows のみ)。
	{
		let weak = ui.as_weak();
		let _ = slint::spawn_local(async move {
			if let Some(ui) = weak.upgrade() {
				if let Ok(winit_window) = ui.window().winit_window().await {
					ergon_slint_controls::cursor::cache_window(winit_window);
				}
			}
		});
		#[cfg(windows)]
		let _ = ergon_slint_controls::cursor::set_file_cursor(
			2,
			std::path::Path::new("assets/hand.cur"),
		);
	}

	// ---------- タイトルバー テーマ (UI と明暗を揃える) ----------
	ergon_slint_controls::platform::apply_titlebar_theme(
		&ui,
		ergon_slint_controls::platform::TitlebarTheme::Light,
	);
	let _ = ergon_slint_controls::platform::applied_titlebar_theme();

	ui.run()
}

/// いまの時刻を "14:05" 形式で返す (セッション一覧の開始時刻)。
fn clock_hhmm() -> String {
	let now = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.as_secs())
		.unwrap_or(0);
	let secs_of_day = (now + 9 * 3600) % 86_400;   // JST 決め打ち (デモ)
	format!("{:02}:{:02}", secs_of_day / 3600, (secs_of_day % 3600) / 60)
}
