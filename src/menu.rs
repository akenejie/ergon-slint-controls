//! メニューのアクセスキー ("&X" 記法) のパース。
//!
//! slint の文字列関数には走査手段 (split / index / slice) がないため、
//! タイトルの "&" 記法の分解はここ (Rust) が担う。描画側は
//! `MenuAccessKey.parse` グローバル純粋コールバック (ui/menu-base.slint) 経由
//! でこの結果を使う。ハンドラの差し込み:
//!
//! ```ignore
//! ui.global::<MenuAccessKey>().on_parse(|title| {
//!     let p = ergon_slint_controls::menu::parse_access_key(&title);
//!     AccessKeyParts {
//!         before: p.before.into(),
//!         key: p.key.into(),
//!         after: p.after.into(),
//!     }
//! });
//! ```
//!
//! 記法は Win32 標準に合わせる:
//! - `&X` → `X` をアクセスキーとして扱う (Alt 押下中は下線)
//! - `&&` → リテラルの `&`
//! - 末尾の孤立した `&` → リテラルの `&`
//! - `&` が無ければアクセスキーなし

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

/// メニュー項目の処理を「再描画 → 次のイベントループ」へ回す。
///
/// メニューを閉じてからネイティブ ダイアログを出す処理を同ターンで実行する
/// と、閉じた描画が行われないままダイアログのネストしたループに入り、
/// 「ポップアップが消えないままメッセージボックスが出る」ように見える。
/// まず再描画を要求し、描画が済むまでの時間を置いてから処理を実行する。
///
/// ```ignore
/// ui.on_menu_item_clicked(move |id| {
///     ergon_slint_controls::menu::defer_action(&ui, move || {
///         // 閉じた描画の後で実行される
///     });
/// });
/// ```
pub fn defer_action<P: slint::ComponentHandle>(
	ui: &P,
	action: impl FnOnce() + 'static,
) {
	ui.window().request_redraw();
	slint::Timer::single_shot(std::time::Duration::from_millis(50), action);
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
