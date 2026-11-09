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
#[cfg(feature = "modal-dialog")]
pub mod modal_dialog;
#[cfg(windows)]
mod owner;
pub mod platform;
#[cfg(target_os = "linux")]
mod ui_font;

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
	/// ui/menu-base.slint / ui/menu.slint / ui/tooltip.slint で export している
	/// グローバルの実体 (アプリの生成コードが SharedGlobals 構築時に読む。
	/// アプリがライブラリを参照すると全エクスポート グローバルが
	/// from_library になるため、生成コードが読むフィールドはこの 6 つで固定。
	/// グローバルを増やすときはここも更新する)。
	pub global_AccessKeyHub: core::pin::Pin<std::rc::Rc<generated::InnerAccessKeyHub>>,
	pub global_MenuAccessKey: core::pin::Pin<std::rc::Rc<generated::InnerMenuAccessKey>>,
	pub global_MenuHintHub: core::pin::Pin<std::rc::Rc<generated::InnerMenuHintHub>>,
	pub global_MenuOpenHub: core::pin::Pin<std::rc::Rc<generated::InnerMenuOpenHub>>,
	pub global_MenuBarTheme: core::pin::Pin<std::rc::Rc<generated::InnerMenuBarTheme>>,
	pub global_TooltipHub: core::pin::Pin<std::rc::Rc<generated::InnerTooltipHub>>,
	inner: sp::Rc<generated::ergon_slint_controlsSharedGlobals>,
}

impl ergon_slint_controlsSharedGlobals {
	/// 生成コードがルート コンポーネントごとに呼ぶ。実体を
	/// [`menu_bar_theme`] へ登録し、[`TooltipHub`](crate::TooltipHub) の
	/// 配置コールバックを配線する。
	pub fn new(root_item_tree_weak: sp::VWeak<sp::ItemTreeVTable>) -> sp::Rc<Self> {
		let inner =
			generated::ergon_slint_controlsSharedGlobals::new(root_item_tree_weak.clone());
		menu_bar_theme::register(root_item_tree_weak.clone(), inner.global_MenuBarTheme.clone());
		wire_tooltip_placement(root_item_tree_weak, &inner.global_TooltipHub);
		sp::Rc::new(Self {
			global_AccessKeyHub: inner.global_AccessKeyHub.clone(),
			global_MenuAccessKey: inner.global_MenuAccessKey.clone(),
			global_MenuHintHub: inner.global_MenuHintHub.clone(),
			global_MenuOpenHub: inner.global_MenuOpenHub.clone(),
			global_MenuBarTheme: inner.global_MenuBarTheme.clone(),
			global_TooltipHub: inner.global_TooltipHub.clone(),
			inner,
		})
	}

	/// 生成コードがルート コンポーネントの生成直後に呼ぶ。内部の init が
	/// MenuBarTheme に既定色を書き込んだ**後**で、取得済みのテーマ色を再適用
	/// する (後から生成されたルート向け。最初のルートはまだ色が無いので
	/// 既定値のまま、setup_window で流し込まれる)。
	///
	/// ここで [`TooltipHub`](crate::TooltipHub) を有効化するのも同じ理由:
	/// 内部の init が全グローバルに宣言既定値を書き込むため、`new()` の時点
	/// で active を true にしておいても init の `active = false` で上書きさ
	/// れる (実測。このバグが「ツールチップが一切出ない」の原因だった)。
	pub fn init_globals(self: &sp::Rc<Self>) {
		self.inner.init_globals();
		self.global_TooltipHub.active.set(true);
		self.global_TooltipHub.debug_log.set(tooltip_debug_enabled());
		menu_bar_theme::reapply_after_init(&self.global_MenuBarTheme);
	}
}

/// [`TooltipHub`](crate::TooltipHub) の配置コールバック (`place`) を配線する。
///
/// ルート コンポーネント (ウィンドウ) ごとの実体に対して呼ばれる。ハンドラで
/// はルートのウィンドウ サイズを実地で読み、[platform の
/// `place_tooltip_axis`](crate::platform::place_tooltip_axis) で「ポインタを
/// パネルの四つ角のいずれかに合わせる」位置を求めて panel-x / panel-y へ書
/// く。表示レイヤ (TooltipPopup) はこれを読むだけ。.slint 側に条件式を書かな
/// ないのは意図的: 子要素のバインディングの三項演算子 (Condition) があると
/// コンポーネント ルートの width: 100% の解決が壊れる (slint コンパイラの
/// default_geometry の挙動。実測)。
///
/// ここで配線するのは setup_window (アプリ側、コンポーネント型ジェネリック)
/// からでは `component.global::<G>()` の Global<P> 実装がジェネリック P に対
/// して得られないため。from_library の実体はルートごとに生成され、この
/// SharedGlobals::new 経由で手に入る。
/// 切り分け用ログの有効フラグ (SLINT_TOOLTIP_DEBUG=1)。
fn tooltip_debug_enabled() -> bool {
	std::env::var("SLINT_TOOLTIP_DEBUG").ok().as_deref() == Some("1")
}

/// [`TooltipHub`](crate::TooltipHub) の配置コールバック (`place`) を配線する。
///
/// ルート コンポーネント (ウィンドウ) ごとの実体に対して呼ばれる。ハンドラは
/// **純粋** (副作用なし): 表示レイヤのジオメトリ バインディングから呼ばれるた
/// め、ウィンドウ サイズを実地で読み、[platform の
/// `place_tooltip_axis`](crate::platform::place_tooltip_axis) で「ポインタを
/// パネルの四つ角のいずれかに合わせる」位置を求めて返すだけ。書き込みを挟ま
/// ないのは意図的: バインディング評価中のハブ書き込みは再レイアウトの連鎖に
/// なり、表示中のちらつきの原因だった (実測)。
fn wire_tooltip_placement(
	root_item_tree_weak: sp::VWeak<sp::ItemTreeVTable>,
	hub: &core::pin::Pin<sp::Rc<generated::InnerTooltipHub>>,
) {
	hub.debug_log.set(tooltip_debug_enabled());
	let tree_weak = root_item_tree_weak.clone();
	// (デバッグコード・コメントアウト) 配置計算の確認用。使うときはコメン
	// トを外す (SLINT_TOOLTIP_DEBUG=1)。
	// let debug_log = tooltip_debug_enabled();
	// ウィンドウ サイズのキャッシュ: place は表示レイヤのジオメトリ バインデ
	// ィングから呼ばれるため、ここで X のラウンドトリップ (window.size() =
	// XGetGeometry) を行うと、ダイアログのマップ / 配置シーケンス中に XSync
	// が走り、WM の配置処理とインターリーブしてちらつきの原因になる (実測)。
	// 最初の呼び出しで 1 度だけ読んでキャッシュし、以後はそれを使う。メイン
	// ウィンドウのリサイズ後は初回の反転判定が古いサイズ基準になる制限があ
	// るが、ツールチップは一時的であり実害は小さい。
	let cache = core::cell::Cell::new(None::<(f32, f32)>);
	hub.place.set_handler(move |(cursor_x, cursor_y, w, h): &(sp::Coord, sp::Coord, sp::Coord, sp::Coord)| {
		let fallback = || {
			generated::PanelPlacement { px: *cursor_x, py: *cursor_y }
		};
		let Some(tree) = tree_weak.upgrade() else { return fallback() };
		let mut adapter = None;
		sp::VRc::borrow_pin(&tree).as_ref().window_adapter(false, &mut adapter);
		let Some(adapter) = adapter else { return fallback() };
		let (win_w, win_h) = match cache.get() {
			Some((win_w, win_h)) => (win_w, win_h),
			None => {
				let window = adapter.window();
				let scale = window.scale_factor();
				let size = window.size();
				let win_w = size.width as f32 / scale;
				let win_h = size.height as f32 / scale;
				cache.set(Some((win_w, win_h)));
				(win_w, win_h)
			}
		};
		let px = crate::platform::place_tooltip_axis(*cursor_x, *w, win_w);
		let py = crate::platform::place_tooltip_axis(*cursor_y, *h, win_h);
		// (デバッグコード・コメントアウト) 配置計算の確認用。使うときはコメ
		// ントを外す (上の debug_log の宣言も一緒に)。
		// if debug_log {
		// 	eprintln!("[tooltip] 配置: cursor=({cursor_x},{cursor_y}) panel-size=({w},{h}) win=({win_w},{win_h}) -> panel=({px},{py})");
		// }
		generated::PanelPlacement { px, py }
	});
}
