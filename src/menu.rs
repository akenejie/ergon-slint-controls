//! メニュー バーの構造登録とキー操作の判定。
//!
//! アプリは `MenuKeyControl` グローバル (ui/menu.slint) へバー項目の並び
//! (title は "&X" 記法) と項目モデルを束ね、`activate` ハンドラに「項目が押
//! されたときの処理」を登録する。キーボード操作の判定は
//! [`handle_menu_key`](menu::handle_menu_key) が担う。アプリ側は項目と処理
//! だけを持ち、キー判定 / Alt 押下管理のコードは持たない。
//!
//! slint の文字列関数には走査手段 (split / index / slice) がないため、
//! タイトルの "&" 記法の分解はここ (Rust) が担う。描画側は
//! `MenuAccessKey.parse` グローバル純粋コールバック (ui/menu-base.slint) 経由
//! でこの結果を使う。ハンドラはライブラリが SharedGlobals の初期化で差し込
//! むため、アプリ側の配線は不要。
//!
//! 記法は Win32 標準に合わせる:
//! - `&X` → `X` をアクセスキーとして扱う (Alt 押下中は下線)
//! - `&&` → リテラルの `&`
//! - 末尾の孤立した `&` → リテラルの `&`
//! - `&` が無ければアクセスキーなし

use crate::{AccessKeyHub, MenuAccessKey, MenuBarMenu, MenuKeyControl, MenuItemView, MenuOpenHub};

/// `before + key + after` は marker を除いた元のタイトルに一致する。
/// `key` が空ならアクセスキーなし。
pub struct AccessKeyParts {
	pub before: String,
	pub key: String,
	pub after: String,
}

/// タイトルの "&" 記法を分解する。
pub fn parse_access_key(title: &str) -> AccessKeyParts {
	let mut before = String::new();
	let mut chars = title.chars().peekable();
	while let Some(c) = chars.next() {
		if c == '&' {
			match chars.peek() {
				Some('&') => {
					chars.next();
					before.push('&');
				}
				Some(&k) => {
					chars.next();
					let after: String = chars.collect();
					return AccessKeyParts {
						before,
						key: k.to_string(),
						after,
					};
				}
				None => before.push('&'),
			}
		} else {
			before.push(c);
		}
	}
	AccessKeyParts { before, key: String::new(), after: String::new() }
}

/// タイトルに埋め込まれたアクセスキー文字 (`&X` の X)。大文字 / 小文字を
/// そのまま返す (比較側で lowercase する)。無ければ `None`。
pub fn access_key_char(title: &str) -> Option<char> {
	parse_access_key(title).key.chars().next()
}

/// 押されたキーの (第一) 文字を小文字で返す。**離し (Released) は無視する**
/// (押下と離しの両方で発火しないよう、呼び出し側はこれ 1 本でよい)。
/// Alt 押下中でも文字はそのまま来る (winit の logical_key)。
///
/// ```ignore
/// // winit フックのキー処理:
/// if let Some(pressed) = ergon_slint_controls::menu::pressed_char(key_event) {
///     // pressed.to_ascii_uppercase() を access_key_char(タイトル) と比較
/// }
/// ```
pub fn pressed_char(event: &slint::winit_030::winit::event::KeyEvent) -> Option<char> {
	if event.state != slint::winit_030::winit::event::ElementState::Pressed {
		return None;
	}
	match &event.logical_key {
		slint::winit_030::winit::keyboard::Key::Character(c) => {
			c.chars().next().map(|c| c.to_ascii_lowercase())
		}
		_ => None,
	}
}

/// メニュー項目の処理のうち、**ブロックするもの (ネイティブ ダイアログな
/// ど) を「再描画 → 次のイベントループ」へ回す**。
///
/// メニューを閉じてからネイティブ ダイアログを出す処理を同ターンで実行する
/// と、閉じた描画が行われないままダイアログのネストしたループに入り、
/// 「ポップアップが消えないままメッセージボックスが出る」ように見える。
/// まず再描画を要求し、描画が済むまでの時間を置いてから処理を実行する。
///
/// **状態を変えるだけの即時処理 (表示の切り替えなど) は defer しない** こと。
/// defer すると「メニューを閉じた描画」と「処理結果の描画」の 2 回の表示
/// 更新が起きて時間ロスになる。即時処理はそのまま実行すれば、メニューを
/// 閉じた更新と同じフレームにまとまる。
///
/// ```ignore
/// ui.on_menu_item_clicked(move |id| {
///     match id.as_str() {
///         "--toggle" => { /* 即時: 閉じた更新と同じフレーム */ }
///         "--settings" => ergon_slint_controls::menu::defer_action(&ui, move || {
///             /* ブロックする処理: 閉じた描画の後に実行 */
///         }),
///         _ => {}
///     }
/// });
/// ```
pub fn defer_action<P: slint::ComponentHandle>(
	ui: &P,
	action: impl FnOnce() + 'static,
) {
	ui.window().request_redraw();
	slint::Timer::single_shot(std::time::Duration::from_millis(50), action);
}

/// メニュー バーのキー操作の判定。アプリの winit イベント フックで
/// KeyboardInput を受けたら**押下 / 離しの両方**をこれへ渡す。
///
/// 戻り値 `true` = メニューが拾った (アプリは処理しない)。
/// `false` = アプリ側の処理を続ける。
///
/// 動作 (Win32 のメニュー バーに合わせる):
/// - **Alt 単体の押下 / 離し**: バーのアクセスキー下線を出すだけ。消費はし
///   ない (false — アプリの処理は続く)。下線状態は `MenuAccessKey.alt-held`
///   が持ち、バー項目はこれに束ねる。
/// - **Alt+文字** (バー項目のタイトルの &X に一致): メニューを開閉し、開いた
///   ときは最初の有効行を選択する。消費する (true)。
/// - **メニュー開閉中**: 文字 (項目の &X に一致) → activate 発火 / Up / Down
///   → 有効行のみの選択移動 (端は折り返す) / Left / Right → メニュー切替 /
///   Enter / Space → 選択行を activate / Esc → 閉じる。いずれも消費する。
/// - それ以外: 消費しない (false)。
///
/// メニューが開いているときの非活性 (Focused(false)) 経路では、アプリが
/// `MenuOpenHub.close()` を呼ぶ (alt-held も一緒に落ちる)。
///
/// ```ignore
/// ui.window().on_winit_window_event(move |_win, event| -> EventResult {
///     if let winit::event::WindowEvent::KeyboardInput { event, .. } = event {
///         if ergon_slint_controls::menu::handle_menu_key(&ui, event) {
///             return EventResult::PreventDefault;
///         }
///         // ... アプリのキー処理 ...
///     }
///     EventResult::Propagate
/// });
/// ```
pub fn handle_menu_key<'a, C: slint::ComponentHandle>(
	ui: &'a C,
	key_event: &slint::winit_030::winit::event::KeyEvent,
) -> bool
where
	MenuKeyControl<'a>: slint::Global<'a, C>,
	MenuAccessKey<'a>: slint::Global<'a, C>,
	MenuOpenHub<'a>: slint::Global<'a, C>,
	AccessKeyHub<'a>: slint::Global<'a, C>,
{
	use slint::Model as _;
	use slint::winit_030::winit::event::ElementState;
	use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};

	let code = match key_event.physical_key {
		PhysicalKey::Code(c) => c,
		_ => return false,
	};

	// Alt 単体の押下 / 離し: 下線を出すだけ (消費しない)。
	if matches!(code, KeyCode::AltLeft | KeyCode::AltRight) {
		let held = key_event.state == ElementState::Pressed;
		ui.global::<MenuAccessKey>().set_alt_held(held);
		return false;
	}
	if key_event.state != ElementState::Pressed {
		return false;
	}

	let registry = ui.global::<MenuKeyControl>();
	let hub = ui.global::<MenuOpenHub>();
	let open = hub.get_open();
	let menus = registry.get_menus();
	let menu_row = |menu: i32| -> Option<MenuBarMenu> {
		if menu < 0 { None } else { menus.row_data(menu as usize) }
	};
	let alt_held = ui.global::<MenuAccessKey>().get_alt_held();
	// 押下中かつ非リピートの文字 (なければ None)。離し / リピートではメニ
	// ューの文字照合を行わない (Alt の離しでの下線解除は上で処理済み)。
	let pressed = if !key_event.repeat && key_event.state == ElementState::Pressed {
		pressed_char(key_event).map(|c| c.to_ascii_uppercase())
	} else {
		None
	};

	// メニューが閉じている間の文字照合。
	// - Alt 押下中はバー項目のアクセスキー (メニューを開く) を最優先。
	// - ラベル (AccessKeyHub 対応) / ボタンのアクセスキーは Alt の有無にかか
	//   わらず照合する (押された文字をハブ / action コールバックへ回す)。
	// - いずれにも一致しなければ消費しない (アプリ固有のショートカットを優
	//   先する)。
	if open < 0 {
		if let Some(pressed) = pressed {
			if alt_held {
				let want = (0..menus.row_count() as i32).find(|&i| {
					menus.row_data(i as usize).map_or(false, |m| {
						access_key_char(&m.title) == Some(pressed)
					})
				});
				if let Some(want) = want {
					hub.set_open(want);
					if let Some(m) = menu_row(want) {
						hub.set_selected_row(first_enabled_row(&m.items));
					}
					return true;
				}
			}
			let label_keys = registry.get_label_keys();
			let label_hit = (0..label_keys.row_count()).any(|i| {
				label_keys.row_data(i).as_deref() == Some(&pressed.to_string())
			});
			if label_hit {
				ui.global::<AccessKeyHub>().invoke_press(pressed.to_string().into());
				return true;
			}
			let action_keys = registry.get_action_keys();
			if let Some(idx) = (0..action_keys.row_count()).find(|&i| {
				action_keys.row_data(i).as_deref() == Some(&pressed.to_string())
			}) {
				registry.invoke_action(idx as i32);
				return true;
			}
		}
		return false;
	}

	// --- メニュー開閉中 (Alt の有無にかかわらず) ---	// 文字: 項目のアクセスキー発火。一致しなければバー項目を照合する
	// (Alt+文字でのメニュー切替)。
	if let Some(pressed) = pressed {
		if let Some(m) = menu_row(open) {
			let row = (0..m.items.row_count()).find(|&i| {
				m.items.row_data(i).map_or(false, |v| {
					v.entry.enabled
						&& !v.entry.is_separator
						&& access_key_char(&v.entry.title) == Some(pressed)
				})
			});
			if let Some(row) = row {
				registry.invoke_activate(open, row as i32);
				hub.invoke_close();
				return true;
			}
		}
		if alt_held {
			let want = (0..menus.row_count() as i32).find(|&i| {
				i != open
					&& menus.row_data(i as usize).map_or(false, |m| {
						access_key_char(&m.title) == Some(pressed)
					})
			});
			if let Some(want) = want {
				hub.set_open(want);
				if let Some(m) = menu_row(want) {
					hub.set_selected_row(first_enabled_row(&m.items));
				}
				return true;
			}
		}
	}

	// Up / Down: 有効行のみの選択移動 (端は折り返す)。
	if matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown) {
		if let Some(m) = menu_row(open) {
			let step: i32 = if matches!(code, KeyCode::ArrowDown) { 1 } else { -1 };
			let mut row = hub.get_selected_row();
			let count = m.items.row_count() as i32;
			// 未選択からの移動は端から始める (Down なら先頭の有効行、Up なら
			// 末尾の有効行)。
			for _ in 0..count {
				row = if row < 0 {
					if step > 0 { 0 } else { count - 1 }
				} else {
					(row + step).rem_euclid(count)
				};
				let enabled = m
					.items
					.row_data(row as usize)
					.map_or(false, |v| v.entry.enabled && !v.entry.is_separator);
				if enabled {
					hub.set_selected_row(row);
					return true;
				}
			}
			return true;
		}
	}

	// Left / Right: メニュー切替 (最初の有効行を選択)。
	if matches!(code, KeyCode::ArrowLeft | KeyCode::ArrowRight) {
		let count = menus.row_count() as i32;
		if count > 0 {
			let next = if matches!(code, KeyCode::ArrowRight) {
				(open + 1).rem_euclid(count)
			} else {
				(open + count - 1).rem_euclid(count)
			};
			hub.set_open(next);
			if let Some(m) = menu_row(next) {
				hub.set_selected_row(first_enabled_row(&m.items));
			}
		}
		return true;
	}

	// Enter / Space: 選択行を activate。
	if matches!(code, KeyCode::Enter | KeyCode::Space) {
		let row = hub.get_selected_row();
		if row >= 0 {
			registry.invoke_activate(open, row);
		}
		hub.invoke_close();
		return true;
	}

	// Esc: 閉じる。
	if matches!(code, KeyCode::Escape) {
		hub.invoke_close();
		return true;
	}

	false
}

/// モデル内の最初の有効行 (区切り / 無効行を読み飛ばす)。無ければ -1。
fn first_enabled_row(items: &slint::ModelRc<MenuItemView>) -> i32 {
	use slint::Model as _;
	(0..items.row_count())
		.find(|&i| {
			items
				.row_data(i)
				.map_or(false, |v| v.entry.enabled && !v.entry.is_separator)
		})
		.map_or(-1, |i| i as i32)
}

/// アプリ共通のショートカット (元に戻す / やり直し / 終了) の判定。
/// Windows / Linux は Ctrl+Z / Ctrl+Y / (閉じるは WM の閉じる操作)、macOS
/// は Cmd+Z / Cmd+Shift+Z / Cmd+Q。winit には OS 差異があり、アプリが差を
/// 吸収しなくてよいようここに置く。

/// アプリ共通ショートカットの修飾キー (Windows / Linux = Ctrl、macOS =
/// Command) が押されているか。ModifiersChanged で受け取った状態をそのまま
/// 渡す。
pub fn is_shortcut_modifier(mods: &slint::winit_030::winit::keyboard::ModifiersState) -> bool {
	#[cfg(target_os = "macos")]
	{
		mods.super_key()
	}
	#[cfg(not(target_os = "macos"))]
	{
		mods.control_key()
	}
}

/// 元に戻す / やり直しの意図。押下 (非リピート) かつショートカット修飾キー
/// ( [`is_shortcut_modifier`](menu::is_shortcut_modifier)) のときだけ意味を
/// 持ち、それ以外は None。`shift` は Shift キーの押下状態 (macOS の
/// Cmd+Shift+Z = redo の判定に使う)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoRedoIntent {
	Undo,
	Redo,
}

pub fn undo_redo_intent(
	event: &slint::winit_030::winit::event::KeyEvent,
	shortcut_mod: bool,
	shift: bool,
) -> Option<UndoRedoIntent> {
	use slint::winit_030::winit::event::ElementState;
	use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};
	if event.state != ElementState::Pressed || event.repeat || !shortcut_mod {
		return None;
	}
	let PhysicalKey::Code(code) = event.physical_key else { return None };
	match code {
		KeyCode::KeyZ => {
			#[cfg(target_os = "macos")]
			{
				// macOS 慣例: Cmd+Z = undo、Cmd+Shift+Z = redo。
				if shift {
					Some(UndoRedoIntent::Redo)
				} else {
					Some(UndoRedoIntent::Undo)
				}
			}
			#[cfg(not(target_os = "macos"))]
			{
				let _ = shift;
				Some(UndoRedoIntent::Undo)
			}
		}
		#[cfg(not(target_os = "macos"))]
		KeyCode::KeyY => Some(UndoRedoIntent::Redo),
		_ => None,
	}
}

/// 終了ショートカット (macOS = Cmd+Q)。他の OS では常に false (終了は WM の
/// 閉じる操作 (Alt+F4 など) として CloseRequested で届く)。
pub fn is_quit_shortcut(
	event: &slint::winit_030::winit::event::KeyEvent,
	shortcut_mod: bool,
) -> bool {
	#[cfg(target_os = "macos")]
	{
		use slint::winit_030::winit::event::ElementState;
		use slint::winit_030::winit::keyboard::{KeyCode, PhysicalKey};
		event.state == ElementState::Pressed
			&& !event.repeat
			&& shortcut_mod
			&& matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyQ))
	}
	#[cfg(not(target_os = "macos"))]
	{
		let _ = (event, shortcut_mod);
		false
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn no_marker() {
		let p = parse_access_key("設定...");
		assert_eq!(p.before, "設定...");
		assert_eq!(p.key, "");
		assert_eq!(p.after, "");
		assert!(access_key_char("設定...").is_none());
	}

	#[test]
	fn marker_in_parens() {
		let p = parse_access_key("設定 (&S)...");
		assert_eq!(p.before, "設定 (");
		assert_eq!(p.key, "S");
		assert_eq!(p.after, ")...");
		assert_eq!(access_key_char("設定 (&S)..."), Some('S'));
	}

	#[test]
	fn leading_marker() {
		let p = parse_access_key("&File");
		assert_eq!(p.before, "");
		assert_eq!(p.key, "F");
		assert_eq!(p.after, "ile");
	}

	#[test]
	fn escaped_ampersand() {
		let p = parse_access_key("A && B &C");
		assert_eq!(p.before, "A & B ");
		assert_eq!(p.key, "C");
		assert_eq!(p.after, "");
	}

	#[test]
	fn trailing_ampersand() {
		let p = parse_access_key("A&");
		assert_eq!(p.before, "A&");
		assert_eq!(p.key, "");
		assert!(access_key_char("A&").is_none());
	}
}
