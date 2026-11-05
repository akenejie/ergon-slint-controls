//! 汎用 Slint 部品集 (クラシックメニューバー / 小ボタン / トグルボタン /
//! チェックボックス / スピンボックス / ツールチップ) と、アプリ非依存の
//! Rust ユーティリティ (ネイティブダイアログ / プラットフォーム操作)。
//! アプリ固有のロジックは含まない。

// サンプル (じかん君) の生成コードが `ergon_slint_controls::…` で本クレート
// を参照できるようにする (外部アプリと同じ from_library 経路にするため。
// build.rs がサンプル用に DEP_* 環境変数を設定してコンパイルする)。
extern crate self as ergon_slint_controls;

pub mod cursor;
pub mod dialogs;
#[cfg(target_os = "linux")]
pub mod gtk;
pub mod menu;
mod menu_bar_theme;
pub mod platform;

// 生成コードをルートに置かず `generated` モジュールへ格納する。下の
// ergon_slint_controlsSharedGlobals ラッパーが本物の SharedGlobals を差し
// 替えるため、生成モジュールの名前を一度隔離してから (glob で) 再公開する。
mod generated {
	include!(concat!(env!("OUT_DIR"), "/lib.rs"));
}
// 生成型 (部品 / 構造体 / グローバル / グローバル内部型) をクレートのルートへ
// 並べる。アプリの生成コードは from_library 機構で
// `ergon_slint_controls::<部品名>` / `ergon_slint_controls::Inner<グローバル名>`
// を直接参照するため、名前はこの位置に無いといけない。
pub use generated::*;

// サンプル (じかん君) の生成コード。ui/timekeeper.slint を外部アプリと同じ
// @ergon_slint_controls import 経路でコンパイルしたもの (build.rs 参照)。
// 外部アプリと同じく、このコンパイル単位のグローバルは from_library で
// ライブラリと共有される。
mod timekeeper_app {
	include!(concat!(env!("OUT_DIR"), "/timekeeper.rs"));
}
pub use timekeeper_app::{SessionEntry, TimeKeeperExport, TimeKeeperWindow};

use slint::private_unstable_api::re_exports as sp;

/// アプリの生成コードが from_library 機構で呼ぶ共有グローバル群の入口。
///
/// 生成コードの `SharedGlobals` をそのまま再公開せず、同じ公開形のラッパーで
/// 差し替えてある。アプリのルート コンポーネント生成時に生成コードから
///
/// - `ergon_slint_controlsSharedGlobals::new(ルート木の弱参照)` が呼ばれ、
///   from_library グローバルの実体が pub フィールド経由で読まれ、
/// - 続いて `init_globals()` が呼ばれる
///
/// ため、この 2 点でライブラリがフックできる。[`menu_bar_theme`] はここで
/// 実体を受け取り、Linux では platform::setup_window の時点で GTK テーマ色
/// (Chromium ui/gtk と同じ方式) を流し込む。アプリ側に必要なコードは無い。
///
/// 公開する形は生成コードと同じ (new / init_globals / from_library グローバ
/// ルの実体フィールド)。
#[allow(non_camel_case_types, non_snake_case)]
pub struct ergon_slint_controlsSharedGlobals {
	/// ui/menu-base.slint / ui/menu.slint で export しているグローバルの実体
	/// (アプリの生成コードが SharedGlobals 構築時に読む。アプリがライブラリを
	/// 参照すると全エクスポート グローバルが from_library になるため、
	/// 生成コードが読むフィールドはこの 5 つで固定。グローバルを増やすときは
	/// ここも更新する)。
	pub global_AccessKeyHub: core::pin::Pin<std::rc::Rc<generated::InnerAccessKeyHub>>,
	pub global_MenuAccessKey: core::pin::Pin<std::rc::Rc<generated::InnerMenuAccessKey>>,
	pub global_MenuHintHub: core::pin::Pin<std::rc::Rc<generated::InnerMenuHintHub>>,
	pub global_MenuOpenHub: core::pin::Pin<std::rc::Rc<generated::InnerMenuOpenHub>>,
	pub global_MenuBarTheme: core::pin::Pin<std::rc::Rc<generated::InnerMenuBarTheme>>,
	inner: sp::Rc<generated::ergon_slint_controlsSharedGlobals>,
}

impl ergon_slint_controlsSharedGlobals {
	/// 生成コードがルート コンポーネントごとに呼ぶ。実体を
	/// [`menu_bar_theme`] へ登録する。
	pub fn new(root_item_tree_weak: sp::VWeak<sp::ItemTreeVTable>) -> sp::Rc<Self> {
		let inner =
			generated::ergon_slint_controlsSharedGlobals::new(root_item_tree_weak.clone());
		menu_bar_theme::register(root_item_tree_weak, inner.global_MenuBarTheme.clone());
		sp::Rc::new(Self {
			global_AccessKeyHub: inner.global_AccessKeyHub.clone(),
			global_MenuAccessKey: inner.global_MenuAccessKey.clone(),
			global_MenuHintHub: inner.global_MenuHintHub.clone(),
			global_MenuOpenHub: inner.global_MenuOpenHub.clone(),
			global_MenuBarTheme: inner.global_MenuBarTheme.clone(),
			inner,
		})
	}

	/// 生成コードがルート コンポーネントの生成直後に呼ぶ。内部の init が
	/// MenuBarTheme に既定色を書き込んだ**後**で、取得済みのテーマ色を再適用
	/// する (後から生成されたルート向け。最初のルートはまだ色が無いので
	/// 既定値のまま、setup_window で流し込まれる)。
	pub fn init_globals(self: &sp::Rc<Self>) {
		self.inner.init_globals();
		menu_bar_theme::reapply_after_init(&self.global_MenuBarTheme);
	}
}
