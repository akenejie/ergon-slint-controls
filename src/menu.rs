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
