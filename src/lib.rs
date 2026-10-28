//! 汎用 Slint 部品集 (クラシックメニューバー / 小ボタン / トグルボタン /
//! チェックボックス / スピンボックス / ツールチップ) と、アプリ非依存の
//! Rust ユーティリティ (ネイティブダイアログ / プラットフォーム操作)。
//! アプリ固有のロジックは含まない。
pub mod cursor;
pub mod dialogs;
#[cfg(target_os = "linux")]
pub mod gtk;
pub mod menu;
pub mod platform;

slint::include_modules!();